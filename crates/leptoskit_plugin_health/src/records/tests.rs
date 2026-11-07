use super::*;
use crate::wire::ABSENT;

/// The common fields of a sample, the way the native handlers write them.
fn header(writer: &mut Writer, id: &str, data_type: DataType, start: i64, end: i64) {
	writer
		.str(id)
		.unwrap()
		.u32(data_type.code())
		.i64(start)
		.i64(end)
		.optional(Some("com.example.watch"))
		.unwrap()
		.optional(None)
		.unwrap()
		.optional(Some("Pixel Watch"))
		.unwrap();
}

fn page(samples: &[Vec<u8>], token: Option<&str>) -> Vec<u8> {
	let mut writer = Writer::default();
	writer.u32(samples.len() as u32);
	let mut bytes = writer.finish();

	for sample in samples {
		bytes.extend_from_slice(sample);
	}

	bytes.extend(Writer::default().optional(token).unwrap().finish());
	bytes
}

fn heart_rate_sample() -> Vec<u8> {
	let mut writer = Writer::default();
	header(&mut writer, "hr-1", DataType::HeartRate, 1_000, 61_000);
	writer.u32(2).i64(1_000).f64(62.0).i64(31_000).f64(64.5);
	writer.finish()
}

fn sleep_sample() -> Vec<u8> {
	let mut writer = Writer::default();
	header(&mut writer, "sleep-1", DataType::Sleep, 0, 3_600_000);
	writer
		.u32(2)
		.i64(0)
		.i64(1_800_000)
		.u32(3)
		.i64(1_800_000)
		.i64(3_600_000)
		.u32(5);
	writer.finish()
}

fn scalar_sample(data_type: DataType, value: f64) -> Vec<u8> {
	let mut writer = Writer::default();
	header(&mut writer, "scalar", data_type, 5, 5);
	writer.f64(value);
	writer.finish()
}

#[test]
fn a_page_of_series_and_sessions_decodes_with_its_sources() {
	let decoded =
		Page::decode(&page(&[heart_rate_sample(), sleep_sample()], Some("next"))).unwrap();
	assert_eq!(decoded.next_page_token.as_deref(), Some("next"));
	let [heart, sleep] = decoded.samples.as_slice() else {
		panic!("two samples expected");
	};
	assert_eq!(heart.id, "hr-1");
	assert_eq!(heart.data_type(), DataType::HeartRate);
	assert_eq!(
		heart.source,
		Source {
			app_id: Some("com.example.watch".into()),
			app_name: None,
			device: Some("Pixel Watch".into()),
		}
	);
	assert_eq!(
		heart.value,
		Value::HeartRate(vec![
			HeartRatePoint {
				time_ms: 1_000,
				bpm: 62.0
			},
			HeartRatePoint {
				time_ms: 31_000,
				bpm: 64.5
			},
		])
	);
	assert_eq!(
		sleep.value,
		Value::Sleep {
			stages: vec![
				SleepStage {
					start_ms: 0,
					end_ms: 1_800_000,
					stage: Stage::Light
				},
				SleepStage {
					start_ms: 1_800_000,
					end_ms: 3_600_000,
					stage: Stage::Rem
				},
			]
		}
	);
	assert_eq!(
		Page::decode(&page(&[], None)).unwrap(),
		Page {
			samples: vec![],
			next_page_token: None
		}
	);
}

#[test]
fn every_scalar_type_keeps_its_own_measure() {
	for (data_type, expected) in [
		(
			DataType::RestingHeartRate,
			Value::RestingHeartRate { bpm: 51.0 },
		),
		(
			DataType::HeartRateVariabilityRmssd,
			Value::HeartRateVariabilityRmssd { milliseconds: 51.0 },
		),
		(
			DataType::HeartRateVariabilitySdnn,
			Value::HeartRateVariabilitySdnn { milliseconds: 51.0 },
		),
		(
			DataType::OxygenSaturation,
			Value::OxygenSaturation { percent: 51.0 },
		),
		(
			DataType::RespiratoryRate,
			Value::RespiratoryRate {
				breaths_per_minute: 51.0,
			},
		),
	] {
		let decoded = Page::decode(&page(&[scalar_sample(data_type, 51.0)], None)).unwrap();
		assert_eq!(decoded.samples[0].value, expected);
		assert_eq!(decoded.samples[0].data_type(), data_type);
	}

	let infinite = page(
		&[scalar_sample(DataType::RespiratoryRate, f64::INFINITY)],
		None,
	);
	assert_eq!(Page::decode(&infinite), Err(Error::InvalidArgument));
}

#[test]
fn steps_and_exercise_decode_and_reject_negative_counts() {
	let mut writer = Writer::default();
	header(&mut writer, "steps", DataType::Steps, 0, 60_000);
	writer.i64(42);
	let steps = writer.finish();
	let mut writer = Writer::default();
	header(&mut writer, "run", DataType::Exercise, 0, 60_000);
	writer
		.u32(2)
		.u32((-7_i32).cast_unsigned())
		.optional(Some("Park run"))
		.unwrap();
	let exercise = writer.finish();
	let decoded = Page::decode(&page(&[steps, exercise], None)).unwrap();
	assert_eq!(decoded.samples[0].value, Value::Steps { count: 42 });
	assert_eq!(
		decoded.samples[1].value,
		Value::Exercise {
			activity: Activity::Running,
			platform_code: -7,
			title: Some("Park run".into()),
		}
	);

	let mut writer = Writer::default();
	header(&mut writer, "steps", DataType::Steps, 0, 60_000);
	writer.i64(-1);
	assert_eq!(
		Page::decode(&page(&[writer.finish()], None)),
		Err(Error::InvalidArgument)
	);
	let mut writer = Writer::default();
	header(&mut writer, "run", DataType::Exercise, 0, 1);
	writer.u32(15).u32(0).optional(None).unwrap();
	assert_eq!(
		Page::decode(&page(&[writer.finish()], None)),
		Err(Error::InvalidArgument)
	);
}

#[test]
fn malformed_samples_are_rejected() {
	let mut backwards = Writer::default();
	header(&mut backwards, "x", DataType::RestingHeartRate, 10, 9);
	backwards.f64(60.0);
	let mut unnamed = Writer::default();
	header(&mut unnamed, "", DataType::RestingHeartRate, 1, 2);
	unnamed.f64(60.0);
	let mut stage = Writer::default();
	header(&mut stage, "s", DataType::Sleep, 0, 10);
	stage.u32(1).i64(0).i64(10).u32(7);
	let mut inverted_stage = Writer::default();
	header(&mut inverted_stage, "s", DataType::Sleep, 0, 10);
	inverted_stage.u32(1).i64(10).i64(0).u32(1);

	for mut sample in [backwards, unnamed, stage, inverted_stage] {
		assert_eq!(
			Page::decode(&page(&[sample.finish()], None)),
			Err(Error::InvalidArgument)
		);
	}

	let mut truncated = heart_rate_sample();
	truncated.pop();
	assert_eq!(
		Page::decode(&page(&[truncated], None)),
		Err(Error::InvalidArgument)
	);
	let mut trailing = page(&[], None);
	trailing.push(0);
	assert_eq!(Page::decode(&trailing), Err(Error::InvalidArgument));
	assert_eq!(
		Page::decode(&[0xFF, 0xFF, 0, 0]),
		Err(Error::InvalidArgument)
	);
}

fn changes(samples: &[Vec<u8>], deleted: &[&str], token: Option<&str>, flags: u32) -> Vec<u8> {
	let mut bytes = page(samples, None);
	bytes.truncate(bytes.len() - 4);
	let mut writer = Writer::default();
	writer.u32(deleted.len() as u32);

	for id in deleted {
		writer.str(id).unwrap();
	}

	writer.optional(token).unwrap().u32(flags);
	bytes.extend(writer.finish());
	bytes
}

#[test]
fn changes_carry_upserts_deletions_and_the_next_token() {
	let decoded = Changes::decode(&changes(&[sleep_sample()], &["gone"], Some("t2"), 2)).unwrap();
	assert_eq!(decoded.upserted.len(), 1);
	assert_eq!(decoded.deleted, ["gone"]);
	assert_eq!(decoded.next_token, Some(ChangesToken::new("t2")));
	assert!(decoded.has_more);
	assert!(!decoded.token_expired);

	let expired = Changes::decode(&changes(&[], &[], None, 1)).unwrap();
	assert!(expired.token_expired);
	assert_eq!(expired.next_token, None);

	for invalid in [
		changes(&[], &[], None, 0),
		changes(&[], &[], Some("t"), 1),
		changes(&[], &[], Some(""), 0),
		changes(&[], &[], Some("t"), 4),
	] {
		assert_eq!(Changes::decode(&invalid), Err(Error::InvalidArgument));
	}
}

#[test]
fn tokens_are_opaque_non_empty_text() {
	let reply = Writer::default().str("abc").unwrap().finish();
	assert_eq!(ChangesToken::decode(&reply), Ok(ChangesToken::new("abc")));
	assert_eq!(ChangesToken::new("abc").as_str(), "abc");
	let empty = Writer::default().str("").unwrap().finish();
	assert_eq!(ChangesToken::decode(&empty), Err(Error::InvalidArgument));
	assert_eq!(
		ChangesToken::decode(&ABSENT.to_le_bytes()),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		changes_request(&ChangesToken::new("abc")).unwrap(),
		[3, 0, 0, 0, b'a', b'b', b'c']
	);
	assert_eq!(
		changes_request(&ChangesToken::new("")),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		changes_request(&ChangesToken::new("x".repeat(64 * 1024 + 1))),
		Err(Error::InvalidArgument)
	);
}

#[test]
fn requests_encode_exactly_and_reject_empty_ranges() {
	let bytes = read_request(DataType::Steps, 1, 2, 500, Some("p")).unwrap();
	let mut expected = vec![4, 0, 0, 0];
	expected.extend(1_i64.to_le_bytes());
	expected.extend(2_i64.to_le_bytes());
	expected.extend(500_u32.to_le_bytes());
	expected.extend([1, 0, 0, 0, b'p']);
	assert_eq!(bytes, expected);
	let first = read_request(DataType::Steps, 1, 2, 1, None).unwrap();
	assert_eq!(first[first.len() - 4..], ABSENT.to_le_bytes());

	for (start, end, size, token) in [
		(2, 2, 1, None),
		(3, 2, 1, None),
		(1, 2, 0, None),
		(1, 2, MAX_PAGE_SIZE + 1, None),
		(1, 2, 1, Some("")),
	] {
		assert_eq!(
			read_request(DataType::Steps, start, end, size, token),
			Err(Error::InvalidArgument)
		);
	}

	assert_eq!(
		types_request(DataTypes::of(&[DataType::Sleep]), 1).unwrap(),
		[0x20, 0, 0, 0, 1, 0, 0, 0]
	);
	assert_eq!(
		types_request(DataTypes::EMPTY, 0),
		Err(Error::InvalidArgument)
	);
}
