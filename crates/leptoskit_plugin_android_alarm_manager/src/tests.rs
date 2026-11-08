use leptoskit_bridge::Plugin as _;

use super::*;

/// Golden bytes shared with `tests/android/AlarmWireTest.kt`.
const ONE_SHOT_REQUEST: &[u8] = &[
	7, 0, 0, 0, // id
	0, // kind: after
	3, 0, 0, 0, // exact | wakeup
	0xdc, 5, 0, 0, 0, 0, 0, 0, // 1500 ms
	0, 0, 0, 0, 0, 0, 0, 0, // no interval
	2, 0, 0, 0, b'h', b'i',
];
const PERIODIC_REQUEST: &[u8] = &[
	0xff, 0xff, 0xff, 0xff, // id -1
	2,    // kind: periodic
	48, 0, 0, 0, // reschedule on reboot | headless
	0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // first: one interval from now
	0x60, 0xea, 0, 0, 0, 0, 0, 0, // 60000 ms
	0, 0, 0, 0,
];
const FIRED: &[u8] = &[
	3, 0, 0, 0, 0, 0, 0, 0, // sequence
	7, 0, 0, 0, // id
	0, 0, 0, 0, 0, 0, 0, 0, // one-shot
	0xe8, 3, 0, 0, 0, 0, 0, 0, // due at 1000
	0xe2, 4, 0, 0, 0, 0, 0, 0, // fired at 1250
	2, 0, 0, 0, b'h', b'i',
];
const STATUS_REPLY: &[u8] = &[35, 0, 0, 0, 1, 0, 2, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0];
const SCHEDULED_REPLY: &[u8] = &[0, 0x00, 0x68, 0xe5, 0xcf, 0x8b, 0x01, 0x00, 0x00];

fn at(millis: i64) -> UnixMillis {
	UnixMillis::new(millis).unwrap()
}

fn fired(sequence: u64) -> FiredAlarm {
	FiredAlarm {
		sequence,
		id: 7,
		interval: None,
		scheduled: at(1000),
		fired: at(1250),
		params: b"hi".to_vec(),
	}
}

fn exact_wakeup() -> Options {
	Options {
		exact: true,
		wakeup: true,
		..Options::default()
	}
}

#[test]
fn requests_match_the_golden_bytes_shared_with_kotlin() {
	let one_shot = Request::one_shot(7, Duration::from_millis(1500))
		.with_options(exact_wakeup())
		.with_params(*b"hi");
	assert_eq!(one_shot.encode().unwrap().as_ref(), ONE_SHOT_REQUEST);
	assert_eq!(Request::decode(ONE_SHOT_REQUEST).unwrap(), one_shot);

	let periodic = Request::periodic(-1, MIN_PERIOD).with_options(Options {
		reschedule_on_reboot: true,
		headless: true,
		..Options::default()
	});
	assert_eq!(periodic.encode().unwrap().as_ref(), PERIODIC_REQUEST);
	assert_eq!(Request::decode(PERIODIC_REQUEST).unwrap(), periodic);
}

#[test]
fn every_schedule_kind_and_id_roundtrips() {
	let max = at(MAX_UNIX_MILLIS);
	for request in [
		Request::one_shot(0, Duration::ZERO),
		Request::one_shot(i32::MIN, MAX_DELAY),
		Request::one_shot_at(i32::MAX, at(0)),
		Request::one_shot_at(1, max),
		Request::periodic(2, MAX_PERIOD),
		Request::periodic(3, MIN_PERIOD).starting_at(at(0)),
		Request::periodic(4, Duration::from_secs(61)).starting_at(max),
		Request::one_shot(5, Duration::from_secs(1)).with_params(vec![0xff; MAX_PARAMS_BYTES]),
	] {
		let bytes = request.encode().unwrap();
		assert_eq!(Request::decode(&bytes).unwrap(), request, "{request:?}");
	}
}

#[test]
fn starting_at_only_changes_periodic_alarms() {
	let one_shot = Request::one_shot(1, Duration::from_secs(1));
	assert_eq!(one_shot.clone().starting_at(at(5)), one_shot);
	assert_eq!(
		Request::periodic(1, MIN_PERIOD).starting_at(at(5)).schedule,
		Schedule::Periodic {
			interval: MIN_PERIOD,
			start: Some(at(5))
		}
	);
}

#[test]
fn durations_outside_their_ranges_are_rejected_before_encoding() {
	for request in [
		Request::one_shot(1, MAX_DELAY + Duration::from_millis(1)),
		Request::one_shot(1, Duration::from_micros(1500)),
		Request::one_shot(1, Duration::MAX),
		Request::periodic(1, Duration::from_millis(59_999)),
		Request::periodic(1, Duration::ZERO),
		Request::periodic(1, MAX_PERIOD + Duration::from_millis(1)),
		Request::periodic(1, MIN_PERIOD + Duration::from_nanos(1)),
	] {
		assert_eq!(request.encode(), Err(Error::InvalidArgument), "{request:?}");
	}
}

#[test]
fn timestamps_are_bounded() {
	assert_eq!(UnixMillis::new(-1), Err(Error::InvalidArgument));
	assert_eq!(UnixMillis::new(i64::MIN), Err(Error::InvalidArgument));
	assert_eq!(
		UnixMillis::new(MAX_UNIX_MILLIS + 1),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		UnixMillis::new(MAX_UNIX_MILLIS).unwrap().get(),
		MAX_UNIX_MILLIS
	);
	assert_eq!(
		UnixMillis::from_system_time(UNIX_EPOCH + Duration::from_micros(1_999)).unwrap(),
		at(1)
	);
	assert_eq!(
		UnixMillis::from_system_time(UNIX_EPOCH - Duration::from_millis(1)),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		UnixMillis::from_system_time(
			UNIX_EPOCH + Duration::from_millis(MAX_UNIX_MILLIS as u64 + 1)
		),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		at(1_700_000_000_123).to_system_time(),
		UNIX_EPOCH + Duration::from_millis(1_700_000_000_123)
	);
}

#[test]
fn option_bits_roundtrip_and_reject_unknown_bits() {
	for bits in 0..128 {
		let decoded = Options::from_bits(bits);
		let fallback_without_exact = bits & 64 != 0 && bits & (1 | 8) == 0;

		if fallback_without_exact {
			assert_eq!(decoded, Err(Error::InvalidArgument), "{bits}");
		} else {
			assert_eq!(decoded.unwrap().bits(), bits, "{bits}");
		}
	}

	for bits in [128, 1 << 31, u32::MAX] {
		assert_eq!(Options::from_bits(bits), Err(Error::InvalidArgument));
	}

	let all = Options {
		exact: true,
		wakeup: true,
		allow_while_idle: true,
		alarm_clock: true,
		reschedule_on_reboot: true,
		headless: true,
		inexact_fallback: true,
	};
	assert_eq!(all.bits(), 127);
	assert_eq!(Options::default().bits(), 0);
}

#[test]
fn inexact_fallback_needs_exact_or_alarm_clock() {
	let fallback = Options {
		inexact_fallback: true,
		..Options::default()
	};
	let request = Request::one_shot(1, Duration::ZERO).with_options(fallback);
	assert_eq!(request.encode(), Err(Error::InvalidArgument));

	for options in [
		Options {
			exact: true,
			..fallback
		},
		Options {
			alarm_clock: true,
			..fallback
		},
	] {
		let request = Request::periodic(1, MIN_PERIOD).with_options(options);
		assert_eq!(
			Request::decode(&request.encode().unwrap()).unwrap(),
			request
		);
	}
}

#[test]
fn params_are_bounded() {
	let request = Request::one_shot(1, Duration::ZERO).with_params(vec![0; MAX_PARAMS_BYTES + 1]);
	assert_eq!(request.encode(), Err(Error::TooLarge));
	let mut bytes = ONE_SHOT_REQUEST[..25].to_vec();
	bytes.extend_from_slice(&u32::try_from(MAX_PARAMS_BYTES + 1).unwrap().to_le_bytes());
	bytes.resize(bytes.len() + MAX_PARAMS_BYTES + 1, 0);
	assert_eq!(Request::decode(&bytes), Err(Error::TooLarge));
	let mut bytes = ONE_SHOT_REQUEST[..25].to_vec();
	bytes.extend_from_slice(&u32::MAX.to_le_bytes());
	assert_eq!(Request::decode(&bytes), Err(Error::InvalidArgument));
}

#[test]
fn malformed_requests_are_rejected() {
	let with = |offset: usize, value: &[u8]| {
		let mut bytes = ONE_SHOT_REQUEST.to_vec();
		bytes[offset..offset + value.len()].copy_from_slice(value);
		bytes
	};
	let mut trailing = ONE_SHOT_REQUEST.to_vec();
	trailing.push(0);

	for bytes in [
		vec![],
		ONE_SHOT_REQUEST[..ONE_SHOT_REQUEST.len() - 1].to_vec(),
		ONE_SHOT_REQUEST[..4].to_vec(),
		trailing,
		// Unknown kind.
		with(4, &[3]),
		with(4, &[0xff]),
		// Unknown option bit.
		with(5, &[0x80]),
		// Negative delay.
		with(9, &(-1_i64).to_le_bytes()),
		// A one-shot alarm with an interval.
		with(17, &60_000_i64.to_le_bytes()),
		// An instant before 1970.
		[
			&with(4, &[1])[..9],
			&(-5_i64).to_le_bytes(),
			&ONE_SHOT_REQUEST[17..],
		]
		.concat(),
		// A param length past the end.
		with(25, &3_u32.to_le_bytes()),
	] {
		assert!(Request::decode(&bytes).is_err(), "{bytes:?}");
	}

	let periodic = |time: i64, interval: i64| {
		let mut bytes = PERIODIC_REQUEST.to_vec();
		bytes[9..17].copy_from_slice(&time.to_le_bytes());
		bytes[17..25].copy_from_slice(&interval.to_le_bytes());
		Request::decode(&bytes)
	};
	assert!(periodic(-1, 60_000).is_ok());
	assert!(periodic(0, 60_000).is_ok());
	assert_eq!(periodic(-2, 60_000), Err(Error::InvalidArgument));
	assert_eq!(
		periodic(MAX_UNIX_MILLIS + 1, 60_000),
		Err(Error::InvalidArgument)
	);
	assert_eq!(periodic(-1, 0), Err(Error::InvalidArgument));
	assert_eq!(periodic(-1, 59_999), Err(Error::InvalidArgument));
	assert_eq!(periodic(-1, -60_000), Err(Error::InvalidArgument));
	assert_eq!(
		periodic(-1, i64::try_from(MAX_PERIOD.as_millis()).unwrap() + 1),
		Err(Error::InvalidArgument)
	);
}

#[test]
fn scheduled_replies_match_golden_bytes_and_reject_bad_codes() {
	let scheduled = Scheduled {
		timing: Timing::Exact,
		first: at(1_700_000_000_000),
	};
	assert_eq!(scheduled.encode(), SCHEDULED_REPLY);
	assert_eq!(Scheduled::decode(SCHEDULED_REPLY).unwrap(), scheduled);

	for timing in [Timing::Exact, Timing::Inexact, Timing::AlarmClock] {
		let reply = Scheduled {
			timing,
			first: at(0),
		};
		assert_eq!(Scheduled::decode(&reply.encode()).unwrap(), reply);
	}

	let mut bad = SCHEDULED_REPLY.to_vec();
	bad[0] = 3;
	assert_eq!(Scheduled::decode(&bad), Err(Error::InvalidArgument));
	let mut negative = SCHEDULED_REPLY.to_vec();
	negative[1..].copy_from_slice(&(-1_i64).to_le_bytes());
	assert_eq!(Scheduled::decode(&negative), Err(Error::InvalidArgument));
	assert!(Scheduled::decode(&SCHEDULED_REPLY[..8]).is_err());
	assert!(Scheduled::decode(&[SCHEDULED_REPLY, &[0]].concat()).is_err());
}

#[test]
fn fired_alarms_match_golden_bytes() {
	assert_eq!(fired(3).encode().unwrap().as_ref(), FIRED);
	assert_eq!(FiredAlarm::decode(FIRED).unwrap(), fired(3));
	let periodic = FiredAlarm {
		interval: Some(MAX_PERIOD),
		..fired(u64::MAX)
	};
	assert_eq!(
		FiredAlarm::decode(&periodic.encode().unwrap()).unwrap(),
		periodic
	);
}

#[test]
fn malformed_fired_alarms_are_rejected() {
	let with = |offset: usize, value: &[u8]| {
		let mut bytes = FIRED.to_vec();
		bytes[offset..offset + value.len()].copy_from_slice(value);
		bytes
	};

	for bytes in [
		// Sequence 0 is never assigned.
		with(0, &[0]),
		// An interval below a minute.
		with(12, &1_000_i64.to_le_bytes()),
		// Times before 1970.
		with(20, &(-1_i64).to_le_bytes()),
		with(28, &(-1_i64).to_le_bytes()),
		FIRED[..FIRED.len() - 1].to_vec(),
		[FIRED, &[0]].concat(),
	] {
		assert_eq!(FiredAlarm::decode(&bytes), Err(Error::InvalidArgument));
	}

	assert_eq!(fired(0).encode(), Err(Error::InvalidArgument));
	assert_eq!(
		FiredAlarm {
			params: vec![0; MAX_PARAMS_BYTES + 1],
			..fired(1)
		}
		.encode(),
		Err(Error::TooLarge)
	);
}

#[test]
fn drain_pages_are_bounded_and_strictly_after_the_cursor() {
	let page = FiredAlarm::encode_page(&[fired(4), fired(9)]).unwrap();
	assert_eq!(
		FiredAlarm::decode_page(&page, 3, 2).unwrap(),
		[fired(4), fired(9)]
	);
	// The page must start after the cursor, increase, and fit the request.
	assert_eq!(
		FiredAlarm::decode_page(&page, 4, 2),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		FiredAlarm::decode_page(&page, 0, 1),
		Err(Error::InvalidArgument)
	);
	let unordered = FiredAlarm::encode_page(&[fired(9), fired(4)]).unwrap();
	assert_eq!(
		FiredAlarm::decode_page(&unordered, 0, 2),
		Err(Error::InvalidArgument)
	);
	let repeated = FiredAlarm::encode_page(&[fired(4), fired(4)]).unwrap();
	assert_eq!(
		FiredAlarm::decode_page(&repeated, 0, 2),
		Err(Error::InvalidArgument)
	);
	let empty = FiredAlarm::encode_page(&[]).unwrap();
	assert_eq!(empty.as_ref(), [0, 0, 0, 0]);
	assert_eq!(FiredAlarm::decode_page(&empty, 0, 0).unwrap(), []);

	let full: Vec<_> = (1..=MAX_FIRED as u64).map(fired).collect();
	let bytes = FiredAlarm::encode_page(&full).unwrap();
	assert_eq!(
		FiredAlarm::decode_page(&bytes, 0, MAX_FIRED as u32).unwrap(),
		full
	);
	let over: Vec<_> = (1..=MAX_FIRED as u64 + 1).map(fired).collect();
	assert_eq!(FiredAlarm::encode_page(&over), Err(Error::TooLarge));
	let mut forged = (MAX_FIRED as u32 + 1).to_le_bytes().to_vec();
	forged.extend_from_slice(&bytes[4..]);
	assert_eq!(
		FiredAlarm::decode_page(&forged, 0, u32::MAX),
		Err(Error::InvalidArgument)
	);
}

#[test]
fn launch_replies_are_absent_or_one_alarm() {
	assert_eq!(FiredAlarm::encode_launch(None).unwrap().as_ref(), [0]);
	assert_eq!(FiredAlarm::decode_launch(&[0]).unwrap(), None);
	let present = FiredAlarm::encode_launch(Some(&fired(3))).unwrap();
	assert_eq!(present[0], 1);
	assert_eq!(&present[1..], FIRED);
	assert_eq!(FiredAlarm::decode_launch(&present).unwrap(), Some(fired(3)));

	for bad in [&[][..], &[2], &[0, 0], &[1]] {
		assert_eq!(FiredAlarm::decode_launch(bad), Err(Error::InvalidArgument));
	}
}

#[test]
fn scheduled_lists_roundtrip_and_reject_repeats() {
	let alarms = [
		ScheduledAlarm {
			id: 1,
			interval: None,
			next: at(5),
			options: exact_wakeup(),
			timing: Timing::Exact,
			params: vec![],
		},
		ScheduledAlarm {
			id: -1,
			interval: Some(MIN_PERIOD),
			next: at(MAX_UNIX_MILLIS),
			options: Options {
				alarm_clock: true,
				inexact_fallback: true,
				..Options::default()
			},
			timing: Timing::Inexact,
			params: vec![1; MAX_PARAMS_BYTES],
		},
	];
	let bytes = ScheduledAlarm::encode_list(&alarms).unwrap();
	assert_eq!(ScheduledAlarm::decode_list(&bytes).unwrap(), alarms);
	assert_eq!(
		ScheduledAlarm::decode_list(&ScheduledAlarm::encode_list(&[]).unwrap()).unwrap(),
		[]
	);

	let repeated = ScheduledAlarm::encode_list(&[alarms[0].clone(), alarms[0].clone()]).unwrap();
	assert_eq!(
		ScheduledAlarm::decode_list(&repeated),
		Err(Error::InvalidArgument)
	);
	let mut too_many = (MAX_ALARMS as u32 + 1).to_le_bytes().to_vec();
	too_many.extend_from_slice(&bytes[4..]);
	assert_eq!(
		ScheduledAlarm::decode_list(&too_many),
		Err(Error::InvalidArgument)
	);
	let mut bad_timing = bytes.to_vec();
	// The first alarm's timing byte follows its id, next, interval and flags.
	bad_timing[4 + 4 + 8 + 8 + 4] = 9;
	assert_eq!(
		ScheduledAlarm::decode_list(&bad_timing),
		Err(Error::InvalidArgument)
	);
	assert!(ScheduledAlarm::decode_list(&bytes[..bytes.len() - 1]).is_err());

	let many: Vec<_> = (0..=MAX_ALARMS as i32)
		.map(|id| {
			ScheduledAlarm {
				id,
				..alarms[0].clone()
			}
		})
		.collect();
	assert_eq!(ScheduledAlarm::encode_list(&many), Err(Error::TooLarge));
	assert!(ScheduledAlarm::encode_list(&many[..MAX_ALARMS]).is_ok());
}

#[test]
fn status_matches_golden_bytes_and_bounds_pending() {
	let status = Status {
		sdk: 35,
		can_schedule_exact: true,
		headless: false,
		pending: 2,
		dropped: 5,
	};
	assert_eq!(status.encode(), STATUS_REPLY);
	assert_eq!(Status::decode(STATUS_REPLY).unwrap(), status);

	let mut bad_flag = STATUS_REPLY.to_vec();
	bad_flag[4] = 2;
	assert_eq!(Status::decode(&bad_flag), Err(Error::InvalidArgument));
	let mut too_many = STATUS_REPLY.to_vec();
	too_many[6..10].copy_from_slice(&(MAX_FIRED as u32 + 1).to_le_bytes());
	assert_eq!(Status::decode(&too_many), Err(Error::InvalidArgument));
	assert!(Status::decode(&STATUS_REPLY[..17]).is_err());
	assert!(Status::decode(&[STATUS_REPLY, &[0]].concat()).is_err());
}

#[test]
fn small_requests_are_exact_length() {
	assert_eq!(encode_cancel(-2), [0xfe, 0xff, 0xff, 0xff]);
	assert_eq!(decode_cancel(&encode_cancel(i32::MIN)).unwrap(), i32::MIN);
	assert!(decode_cancel(&[0, 0, 0]).is_err());
	assert!(decode_cancel(&[0, 0, 0, 0, 0]).is_err());

	assert_eq!(decode_next(&encode_next(u64::MAX)).unwrap(), u64::MAX);
	assert_eq!(decode_next(&encode_next(0)).unwrap(), 0);
	assert!(decode_next(&[0; 7]).is_err());
	assert!(decode_next(&[0; 9]).is_err());

	assert_eq!(
		encode_drain(3, 2).unwrap(),
		[3, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0]
	);
	assert_eq!(decode_drain(&encode_drain(9, 0).unwrap()).unwrap(), (9, 0));
	assert_eq!(
		encode_drain(0, MAX_FIRED as u32 + 1),
		Err(Error::InvalidArgument)
	);
	let mut over = [0; 12];
	over[8..].copy_from_slice(&(MAX_FIRED as u32 + 1).to_le_bytes());
	assert_eq!(decode_drain(&over), Err(Error::InvalidArgument));
	assert!(decode_drain(&[0; 11]).is_err());

	assert_eq!(encode_finish(0), Err(Error::InvalidArgument));
	assert_eq!(decode_finish(&encode_finish(5).unwrap()).unwrap(), 5);
	assert_eq!(decode_finish(&[0; 8]), Err(Error::InvalidArgument));
	assert!(decode_finish(&[1; 9]).is_err());

	assert!(!decode_flag(&[0]).unwrap());
	assert!(decode_flag(&[1]).unwrap());

	for bad in [&[][..], &[2], &[1, 0]] {
		assert_eq!(decode_flag(bad), Err(Error::InvalidArgument));
	}
}

#[test]
fn unsupported_hosts_validate_then_report_unavailable() {
	let mut plugin = Unsupported;
	assert_eq!(plugin.name(), NAME);
	assert_eq!(plugin.methods(), METHODS);

	for (method, payload) in [
		(STATUS, Bytes::new()),
		(LIST, Bytes::new()),
		(REQUEST_EXACT_PERMISSION, Bytes::new()),
		(LAUNCH, Bytes::new()),
		(SCHEDULE, Bytes::from_static(ONE_SHOT_REQUEST)),
		(CANCEL, Bytes::copy_from_slice(&encode_cancel(1))),
		(NEXT, Bytes::copy_from_slice(&encode_next(0))),
		(DRAIN, Bytes::copy_from_slice(&encode_drain(0, 1).unwrap())),
		(FINISH, Bytes::copy_from_slice(&encode_finish(1).unwrap())),
		(MethodId(99), Bytes::new()),
	] {
		assert_eq!(plugin.invoke(method, payload), Err(Error::Unavailable));
	}

	for (method, payload) in [
		(STATUS, Bytes::from_static(&[0])),
		(LAUNCH, Bytes::from_static(&[0])),
		(SCHEDULE, Bytes::from_static(&[0])),
		(CANCEL, Bytes::new()),
		(NEXT, Bytes::new()),
		(DRAIN, Bytes::new()),
		(FINISH, Bytes::from_static(&[0; 8])),
	] {
		assert_eq!(
			plugin.invoke(method, payload),
			Err(Error::InvalidArgument),
			"{method:?}"
		);
	}
}

#[test]
fn method_ids_are_stable_and_distinct() {
	let ids: Vec<_> = METHODS.iter().map(|method| method.0).collect();
	assert_eq!(ids, (0..9).collect::<Vec<_>>());
	assert_eq!(NAME, "android_alarm_manager");
}
