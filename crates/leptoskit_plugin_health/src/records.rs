//! Samples, pages and change sets as the native handlers encode them.

use leptoskit_bridge::Error;

use crate::DataType;
use crate::DataTypes;
use crate::wire::Reader;
use crate::wire::Writer;

/// The most samples one read may ask for.
pub const MAX_PAGE_SIZE: u32 = 1000;

/// Where a sample came from, as far as the platform says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Source {
	/// The package name (Android) or bundle identifier (iOS) of the app that
	/// wrote it.
	pub app_id: Option<String>,
	/// The source's display name. iOS only; it is often the device's name,
	/// such as "Apple Watch".
	pub app_name: Option<String>,
	/// The recording device, such as "Google Pixel Watch 3".
	pub device: Option<String>,
}

/// One heart-rate measurement in a series.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeartRatePoint {
	/// When it was measured, in Unix milliseconds.
	pub time_ms: i64,
	/// Beats per minute.
	pub bpm: f64,
}

/// A sleep stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
	/// The platform recorded sleep without saying which stage.
	Unknown,
	/// Awake, including awake in bed and out of bed.
	Awake,
	/// Asleep, stage unspecified.
	Asleep,
	/// Light (iOS "core") sleep.
	Light,
	/// Deep sleep.
	Deep,
	/// REM sleep.
	Rem,
	/// In bed, without saying whether asleep. iOS only.
	InBed,
}

impl Stage {
	fn decode(code: u32) -> Result<Self, Error> {
		Ok(match code {
			0 => Self::Unknown,
			1 => Self::Awake,
			2 => Self::Asleep,
			3 => Self::Light,
			4 => Self::Deep,
			5 => Self::Rem,
			6 => Self::InBed,
			_ => return Err(Error::InvalidArgument),
		})
	}
}

/// One stage of a sleep session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SleepStage {
	/// Start, in Unix milliseconds.
	pub start_ms: i64,
	/// End, in Unix milliseconds.
	pub end_ms: i64,
	/// The stage.
	pub stage: Stage,
}

/// The common activities, so one app can treat both platforms alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Activity {
	/// Anything else; [`Value::Exercise`] keeps the platform's own code.
	Other,
	/// Walking.
	Walking,
	/// Running, outdoors or on a treadmill.
	Running,
	/// Cycling, outdoors or stationary.
	Cycling,
	/// Swimming, in a pool or open water.
	Swimming,
	/// Hiking.
	Hiking,
	/// Strength or weight training.
	StrengthTraining,
	/// Yoga.
	Yoga,
	/// High-intensity interval training.
	HighIntensityIntervalTraining,
	/// Rowing, on water or a machine.
	Rowing,
	/// The elliptical trainer.
	Elliptical,
	/// Stair climbing.
	StairClimbing,
	/// Dancing.
	Dancing,
	/// Pilates.
	Pilates,
	/// Martial arts.
	MartialArts,
}

impl Activity {
	const ALL: [Self; 15] = [
		Self::Other,
		Self::Walking,
		Self::Running,
		Self::Cycling,
		Self::Swimming,
		Self::Hiking,
		Self::StrengthTraining,
		Self::Yoga,
		Self::HighIntensityIntervalTraining,
		Self::Rowing,
		Self::Elliptical,
		Self::StairClimbing,
		Self::Dancing,
		Self::Pilates,
		Self::MartialArts,
	];

	fn decode(code: u32) -> Result<Self, Error> {
		Self::ALL
			.get(code as usize)
			.copied()
			.ok_or(Error::InvalidArgument)
	}
}

/// A sample's measurement, which also says its data type.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
	/// Heart-rate measurements within the sample's interval. iOS samples hold
	/// one; Health Connect records can hold many.
	HeartRate(Vec<HeartRatePoint>),
	/// Resting heart rate.
	RestingHeartRate {
		/// Beats per minute.
		bpm: f64,
	},
	/// Heart rate variability as RMSSD (Health Connect).
	HeartRateVariabilityRmssd {
		/// Milliseconds.
		milliseconds: f64,
	},
	/// Heart rate variability as SDNN (`HealthKit`).
	HeartRateVariabilitySdnn {
		/// Milliseconds.
		milliseconds: f64,
	},
	/// Steps over the sample's interval.
	Steps {
		/// Steps counted.
		count: u64,
	},
	/// A sleep session (Health Connect) or one stage interval (`HealthKit`,
	/// which has no sessions; its stages arrive as separate samples).
	Sleep {
		/// The stages, in time order; empty when none were recorded.
		stages: Vec<SleepStage>,
	},
	/// Blood oxygen saturation.
	OxygenSaturation {
		/// Percent, 0 to 100.
		percent: f64,
	},
	/// Respiratory rate.
	RespiratoryRate {
		/// Breaths per minute.
		breaths_per_minute: f64,
	},
	/// An exercise session; the sample's interval is when it ran.
	Exercise {
		/// The activity, when it is one of the common ones.
		activity: Activity,
		/// The platform's own activity code: Health Connect's exercise type
		/// or `HealthKit`'s workout activity type.
		platform_code: i32,
		/// The session's title, when the recording app gave one.
		title: Option<String>,
	},
}

impl Value {
	/// The data type this value belongs to.
	pub fn data_type(&self) -> DataType {
		match self {
			Self::HeartRate(_) => DataType::HeartRate,
			Self::RestingHeartRate { .. } => DataType::RestingHeartRate,
			Self::HeartRateVariabilityRmssd { .. } => DataType::HeartRateVariabilityRmssd,
			Self::HeartRateVariabilitySdnn { .. } => DataType::HeartRateVariabilitySdnn,
			Self::Steps { .. } => DataType::Steps,
			Self::Sleep { .. } => DataType::Sleep,
			Self::OxygenSaturation { .. } => DataType::OxygenSaturation,
			Self::RespiratoryRate { .. } => DataType::RespiratoryRate,
			Self::Exercise { .. } => DataType::Exercise,
		}
	}

	fn decode(data_type: DataType, reader: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(match data_type {
			DataType::HeartRate => Self::HeartRate(heart_rate(reader)?),
			DataType::RestingHeartRate => Self::RestingHeartRate { bpm: reader.f64()? },
			DataType::HeartRateVariabilityRmssd => {
				Self::HeartRateVariabilityRmssd {
					milliseconds: reader.f64()?,
				}
			}
			DataType::HeartRateVariabilitySdnn => {
				Self::HeartRateVariabilitySdnn {
					milliseconds: reader.f64()?,
				}
			}
			DataType::Steps => {
				Self::Steps {
					count: u64::try_from(reader.i64()?).map_err(|_| Error::InvalidArgument)?,
				}
			}
			DataType::Sleep => {
				Self::Sleep {
					stages: sleep(reader)?,
				}
			}
			DataType::OxygenSaturation => {
				Self::OxygenSaturation {
					percent: reader.f64()?,
				}
			}
			DataType::RespiratoryRate => {
				Self::RespiratoryRate {
					breaths_per_minute: reader.f64()?,
				}
			}
			DataType::Exercise => {
				Self::Exercise {
					activity: Activity::decode(reader.u32()?)?,
					platform_code: reader.u32()?.cast_signed(),
					title: reader.optional()?.map(str::to_owned),
				}
			}
		})
	}
}

fn heart_rate(reader: &mut Reader<'_>) -> Result<Vec<HeartRatePoint>, Error> {
	let count = reader.count(16)?;
	let mut points = Vec::with_capacity(count);

	for _ in 0..count {
		points.push(HeartRatePoint {
			time_ms: reader.i64()?,
			bpm: reader.f64()?,
		});
	}

	Ok(points)
}

fn sleep(reader: &mut Reader<'_>) -> Result<Vec<SleepStage>, Error> {
	let count = reader.count(20)?;
	let mut stages = Vec::with_capacity(count);

	for _ in 0..count {
		let stage = SleepStage {
			start_ms: reader.i64()?,
			end_ms: reader.i64()?,
			stage: Stage::decode(reader.u32()?)?,
		};

		if stage.end_ms < stage.start_ms {
			return Err(Error::InvalidArgument);
		}

		stages.push(stage);
	}

	Ok(stages)
}

/// One health record.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
	/// The platform's identifier for the record, stable across reads and the
	/// identifier [`Changes::deleted`] names.
	pub id: String,
	/// Start, in Unix milliseconds.
	pub start_ms: i64,
	/// End, in Unix milliseconds; equal to the start for instantaneous
	/// measurements.
	pub end_ms: i64,
	/// Where it came from.
	pub source: Source,
	/// The measurement.
	pub value: Value,
}

/// The fewest bytes one encoded sample takes, which bounds sample counts.
const MIN_SAMPLE: usize = 4 + 4 + 8 + 8 + 4 * 3 + 8;

impl Sample {
	/// The sample's data type.
	pub fn data_type(&self) -> DataType {
		self.value.data_type()
	}

	fn decode(reader: &mut Reader<'_>) -> Result<Self, Error> {
		let id = reader.str()?.to_owned();
		let data_type = DataType::from_code(reader.u32()?)?;
		let start_ms = reader.i64()?;
		let end_ms = reader.i64()?;

		if id.is_empty() || end_ms < start_ms {
			return Err(Error::InvalidArgument);
		}

		let source = Source {
			app_id: reader.optional()?.map(str::to_owned),
			app_name: reader.optional()?.map(str::to_owned),
			device: reader.optional()?.map(str::to_owned),
		};
		Ok(Self {
			id,
			start_ms,
			end_ms,
			source,
			value: Value::decode(data_type, reader)?,
		})
	}
}

fn samples(reader: &mut Reader<'_>) -> Result<Vec<Sample>, Error> {
	let count = reader.count(MIN_SAMPLE)?;
	let mut samples = Vec::with_capacity(count);

	for _ in 0..count {
		samples.push(Sample::decode(reader)?);
	}

	Ok(samples)
}

/// One page of a read.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
	/// The samples, oldest first.
	pub samples: Vec<Sample>,
	/// Pass this to the next read for the following page; `None` on the last
	/// page.
	pub next_page_token: Option<String>,
}

impl Page {
	/// Decodes a read reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut reader = Reader::new(payload);
		let samples = samples(&mut reader)?;
		let next_page_token = reader.optional()?.map(str::to_owned);
		reader.finish()?;
		Ok(Self {
			samples,
			next_page_token,
		})
	}
}

/// Where incremental sync left off. Store it as text and pass it back to
/// [`changes`](crate::guest::Health::changes); its contents are the
/// platform's and may change between versions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChangesToken(String);

impl ChangesToken {
	/// Wraps a token stored earlier.
	pub fn new(token: impl Into<String>) -> Self {
		Self(token.into())
	}

	/// The token's text, to store.
	pub fn as_str(&self) -> &str {
		&self.0
	}

	/// Decodes a changes-token reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut reader = Reader::new(payload);
		let token = reader.str()?;
		reader.finish()?;
		(!token.is_empty())
			.then(|| Self(token.to_owned()))
			.ok_or(Error::InvalidArgument)
	}
}

/// What changed since a token.
#[derive(Debug, Clone, PartialEq)]
pub struct Changes {
	/// Samples written or updated since the token.
	pub upserted: Vec<Sample>,
	/// Identifiers of samples deleted since the token.
	pub deleted: Vec<String>,
	/// Where to continue from; `None` when the token expired.
	pub next_token: Option<ChangesToken>,
	/// Whether more changes are waiting: call again with `next_token`.
	pub has_more: bool,
	/// The token is too old (Health Connect keeps changes for 30 days). Read
	/// the data again and start over from a new
	/// [`changes_token`](crate::guest::Health::changes_token).
	pub token_expired: bool,
}

impl Changes {
	/// Decodes a changes reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut reader = Reader::new(payload);
		let upserted = samples(&mut reader)?;
		let count = reader.count(4)?;
		let mut deleted = Vec::with_capacity(count);

		for _ in 0..count {
			deleted.push(reader.str()?.to_owned());
		}

		let next_token = reader.optional()?;
		let flags = reader.u32()?;
		reader.finish()?;
		let token_expired = flags & 1 != 0;

		if flags & !3 != 0 || token_expired == next_token.is_some() || next_token == Some("") {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			upserted,
			deleted,
			next_token: next_token.map(ChangesToken::new),
			has_more: flags & 2 != 0,
			token_expired,
		})
	}
}

/// The request for one page of samples.
pub fn read_request(
	data_type: DataType,
	start_ms: i64,
	end_ms: i64,
	page_size: u32,
	page_token: Option<&str>,
) -> Result<Vec<u8>, Error> {
	if start_ms >= end_ms || !(1..=MAX_PAGE_SIZE).contains(&page_size) || page_token == Some("") {
		return Err(Error::InvalidArgument);
	}

	Ok(Writer::default()
		.u32(data_type.code())
		.i64(start_ms)
		.i64(end_ms)
		.u32(page_size)
		.optional(page_token)?
		.finish())
}

/// The request for a permission prompt or a changes token: a non-empty
/// set of types and the extra flags.
pub fn types_request(types: DataTypes, flags: u32) -> Result<Vec<u8>, Error> {
	if types.is_empty() {
		return Err(Error::InvalidArgument);
	}

	Ok(Writer::default().u32(types.bits()).u32(flags).finish())
}

/// The request for the changes after `token`.
pub fn changes_request(token: &ChangesToken) -> Result<Vec<u8>, Error> {
	if token.0.is_empty() || token.0.len() > 64 * 1024 {
		return Err(Error::InvalidArgument);
	}

	Ok(Writer::default().str(&token.0)?.finish())
}

#[cfg(test)]
mod tests;
