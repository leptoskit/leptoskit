//! The batch format: its exact bytes, round trips, hostile input, and how
//! small it is next to the JSON `PostHog`'s browser library sends.

use std::fmt::Write as _;
use std::io::Write as _;

use leptoskit_analytics::wire;
use leptoskit_analytics::wire::Batch;
use leptoskit_analytics::wire::Body;
use leptoskit_analytics::wire::Entry;
use leptoskit_analytics::wire::Value;
use proptest::prelude::*;

const SENT_AT: i64 = 1_762_646_400_000;

fn uuid(n: u8) -> [u8; 16] {
	wire::uuid_v7(SENT_AT - 1_000 + i64::from(n), [n; 10])
}

fn web_context() -> wire::Properties {
	vec![
		("$lib".into(), Value::Text("leptoskit-web".into())),
		("$lib_version".into(), Value::Text("0.3.0".into())),
		("$os".into(), Value::Text("Mac OS X".into())),
		("$browser".into(), Value::Text("Safari".into())),
		("$screen_width".into(), Value::Int(1512)),
		("$screen_height".into(), Value::Int(982)),
		("$locale".into(), Value::Text("en-GB".into())),
		("$timezone".into(), Value::Text("Europe/London".into())),
	]
}

fn pageview(n: u8) -> Entry {
	Entry {
		uuid: uuid(n),
		distinct_id: "019a5f3c-anon".into(),
		session_id: Some([7; 16]),
		context: 0,
		body: Body::Event {
			name: "$pageview".into(),
			properties: vec![
				(
					"$current_url".into(),
					Value::Text(format!("https://app.example/notes/{n}")),
				),
				("$pathname".into(), Value::Text(format!("/notes/{n}"))),
				("$lk_route".into(), Value::Text("/notes/:id".into())),
			],
		},
	}
}

fn sample() -> Batch {
	Batch {
		sent_at: SENT_AT,
		contexts: vec![web_context()],
		entries: vec![
			pageview(1),
			Entry {
				uuid: uuid(2),
				distinct_id: "019a5f3c-anon".into(),
				session_id: Some([7; 16]),
				context: 0,
				body: Body::Event {
					name: "checkout_completed".into(),
					properties: vec![
						("plan".into(), Value::Text("pro".into())),
						("amount_cents".into(), Value::Int(1999)),
						("coupon".into(), Value::Null),
						("trial".into(), Value::Bool(false)),
						("ratio".into(), Value::Float(0.25)),
						("at".into(), Value::Time(SENT_AT)),
						("tags".into(), Value::List(vec![Value::Text("a".into())])),
						(
							"meta".into(),
							Value::Map(vec![("plan".into(), Value::Int(2))]),
						),
					],
				},
			},
			Entry {
				uuid: uuid(3),
				distinct_id: "user-42".into(),
				session_id: Some([7; 16]),
				context: 0,
				body: Body::Identify {
					anon_distinct_id: "019a5f3c-anon".into(),
					set: vec![("email".into(), Value::Text("ada@example.com".into()))],
					set_once: vec![],
				},
			},
			Entry {
				uuid: uuid(4),
				distinct_id: "user-42".into(),
				session_id: None,
				context: 0,
				body: Body::Alias {
					alias: "ada".into(),
				},
			},
			Entry {
				uuid: uuid(5),
				distinct_id: "user-42".into(),
				session_id: None,
				context: 0,
				body: Body::Set {
					set: vec![("plan".into(), Value::Text("pro".into()))],
					set_once: vec![("first_plan".into(), Value::Text("free".into()))],
					unset: vec!["trial_ends".into()],
				},
			},
			Entry {
				uuid: uuid(6),
				distinct_id: "user-42".into(),
				session_id: None,
				context: 0,
				body: Body::GroupIdentify {
					group_type: "company".into(),
					group_key: "acme".into(),
					set: vec![("name".into(), Value::Text("Acme".into()))],
				},
			},
		],
	}
}

fn hex(bytes: &[u8]) -> String {
	bytes
		.chunks(32)
		.map(|line| {
			line.iter().fold(String::new(), |mut out, byte| {
				let _ = write!(out, "{byte:02x}");
				out
			})
		})
		.collect::<Vec<_>>()
		.join("\n")
}

#[test]
fn the_encoding_is_stable() {
	insta::assert_snapshot!(hex(&wire::encode(&sample())));
}

#[test]
fn a_batch_round_trips() {
	let batch = sample();
	let decoded = wire::decode(&wire::encode(&batch), &wire::Limits::default()).unwrap();
	assert_eq!(decoded, batch);
}

#[test]
fn truncated_batches_are_refused_not_misread() {
	let bytes = wire::encode(&sample());
	for len in 0..bytes.len() {
		assert!(
			wire::decode(&bytes[..len], &wire::Limits::default()).is_err(),
			"{len} bytes decoded"
		);
	}
	let mut longer = bytes.clone();
	longer.push(0);
	assert_eq!(
		wire::decode(&longer, &wire::Limits::default())
			.unwrap_err()
			.reason,
		"bytes after the batch's end"
	);
}

#[test]
fn limits_bound_what_a_batch_can_hold() {
	let limits = wire::Limits {
		entries: 2,
		..wire::Limits::default()
	};
	let error = wire::decode(&wire::encode(&sample()), &limits).unwrap_err();
	assert_eq!(error.reason, "too many entries");

	let mut nested = Value::Null;
	for _ in 0..12 {
		nested = Value::List(vec![nested]);
	}
	let mut batch = sample();
	batch.entries.truncate(1);
	batch.entries[0].body = Body::Event {
		name: "deep".into(),
		properties: vec![("x".into(), nested)],
	};
	let error = wire::decode(&wire::encode(&batch), &wire::Limits::default()).unwrap_err();
	assert_eq!(error.reason, "lists and maps nested too deeply");
}

#[test]
fn a_huge_count_in_a_tiny_batch_is_refused_before_allocating() {
	// "LKE", version 1, sent_at 0, then a string count of 2^63.
	let mut bytes = b"LKE\x01\x00".to_vec();
	bytes.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F]);
	assert_eq!(
		wire::decode(&bytes, &wire::Limits::default())
			.unwrap_err()
			.reason,
		"too many strings"
	);
}

fn value() -> impl Strategy<Value = Value> {
	let leaf = prop_oneof![
		Just(Value::Null),
		any::<bool>().prop_map(Value::Bool),
		any::<i64>().prop_map(Value::Int),
		any::<f64>()
			.prop_filter("NaN never equals itself", |float| !float.is_nan())
			.prop_map(Value::Float),
		".{0,20}".prop_map(Value::Text),
		any::<i64>().prop_map(Value::Time),
	];
	leaf.prop_recursive(3, 16, 4, |inner| {
		prop_oneof![
			prop::collection::vec(inner.clone(), 0..4).prop_map(Value::List),
			prop::collection::vec(("[a-z$]{1,8}", inner), 0..4).prop_map(Value::Map),
		]
	})
}

fn entry() -> impl Strategy<Value = Entry> {
	(
		any::<[u8; 16]>(),
		prop_oneof!["user-[0-9]", ".{1,12}"],
		prop::option::of(prop_oneof![Just([1_u8; 16]), any::<[u8; 16]>()]),
		"[a-z$_]{1,12}",
		prop::collection::vec(("[a-z$_]{1,8}", value()), 0..5),
	)
		.prop_map(|(uuid, distinct_id, session_id, name, properties)| {
			Entry {
				uuid,
				distinct_id,
				session_id,
				context: 0,
				body: Body::Event { name, properties },
			}
		})
}

proptest! {
	#[test]
	fn any_batch_round_trips(entries in prop::collection::vec(entry(), 0..20), sent_at in 0_i64..i64::MAX) {
		let batch = Batch { sent_at, contexts: vec![web_context()], entries };
		let decoded = wire::decode(&wire::encode(&batch), &wire::Limits::default()).unwrap();
		prop_assert_eq!(decoded, batch);
	}

	#[test]
	fn any_bytes_decode_or_fail_without_panicking(mut bytes in prop::collection::vec(any::<u8>(), 0..256)) {
		let _ = wire::decode(&bytes, &wire::Limits::default());
		if bytes.len() >= 4 {
			bytes[..4].copy_from_slice(b"LKE\x01");
		}
		let _ = wire::decode(&bytes, &wire::Limits::default());
	}
}

fn gzip(bytes: &[u8]) -> usize {
	let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
	encoder.write_all(bytes).unwrap();
	encoder.finish().unwrap().len()
}

/// Twenty pageviews, as posthog-js would send them: every event carries the
/// properties `getEventProperties` and the core add (see
/// `docs/agents/analytics-research/posthog.md`), with values from a real
/// browser.
fn posthog_json(count: u8) -> String {
	let events: Vec<String> = (1..=count)
		.map(|n| {
			format!(
				r#"{{"uuid":"019a5f3c-0000-7000-8000-0000000000{n:02}","event":"$pageview","properties":{{"$os":"Mac OS X","$os_version":"15.1.0","$browser":"Safari","$device_type":"Desktop","$current_url":"https://app.example/notes/{n}","$host":"app.example","$pathname":"/notes/{n}","$raw_user_agent":"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Safari/605.1.15","$browser_version":18.1,"$browser_language":"en-GB","$browser_language_prefix":"en","$screen_height":982,"$screen_width":1512,"$viewport_height":859,"$viewport_width":1512,"$lib":"web","$lib_version":"1.200.0","$insert_id":"k3j4h5g6f7d8s9a0","$time":1762646399.{n:03},"distinct_id":"019a5f3c-anon","$device_id":"019a5f3c-anon","$timezone":"Europe/London","$timezone_offset":0,"$session_id":"019a5f3c-0000-7000-8000-000000000777","$window_id":"019a5f3c-0000-7000-8000-000000000888","$referrer":"$direct","$referring_domain":"$direct","$is_identified":false,"$process_person_profile":false,"token":"phc_0123456789abcdefghijklmnopqrstuvwxyzABCDE"}},"timestamp":"2025-11-09T00:00:00.{n:03}Z"}}"#
			)
		})
		.collect();
	format!(
		r#"{{"api_key":"phc_0123456789abcdefghijklmnopqrstuvwxyzABCDE","batch":[{}],"sent_at":"2025-11-09T00:00:01.000Z"}}"#,
		events.join(",")
	)
}

#[test]
fn twenty_pageviews_are_far_smaller_than_posthogs_json() {
	let ours = Batch {
		sent_at: SENT_AT,
		contexts: vec![web_context()],
		entries: (1..=20).map(pageview).collect(),
	};
	let ours = wire::encode(&ours);
	let theirs = posthog_json(20);
	let report = format!(
		"posthog-js JSON: {} bytes, {} gzipped ({} per event)\nleptoskit batch: {} bytes, {} \
		 gzipped ({} per event)",
		theirs.len(),
		gzip(theirs.as_bytes()),
		gzip(theirs.as_bytes()) / 20,
		ours.len(),
		gzip(&ours),
		gzip(&ours) / 20,
	);
	insta::assert_snapshot!(report);
	assert!(gzip(&ours) / 20 < 100, "{report}");
}

#[test]
fn repeated_interned_strings_cannot_expand_past_the_memory_budget() {
	let entry = Entry {
		uuid: wire::uuid_v7(1_000, [1; 10]),
		distinct_id: "x".repeat(10_000),
		session_id: None,
		context: 0,
		body: Body::Event {
			name: "event".into(),
			properties: vec![],
		},
	};
	let encoded = wire::encode(&Batch {
		sent_at: 1_000,
		contexts: vec![],
		entries: vec![entry; 100],
	});
	assert!(encoded.len() < 15_000);
	let limits = wire::Limits {
		decoded_bytes: 100_000,
		..Default::default()
	};
	assert_eq!(
		wire::decode(&encoded, &limits).unwrap_err().reason,
		"decoded batch exceeds the memory limit"
	);
}

#[test]
fn property_names_also_count_towards_the_decoded_memory_budget() {
	let batch = Batch {
		sent_at: SENT_AT,
		contexts: vec![],
		entries: vec![Entry {
			uuid: wire::uuid_v7(SENT_AT, [1; 10]),
			distinct_id: "visitor".into(),
			session_id: None,
			context: 0,
			body: Body::Event {
				name: "event".into(),
				properties: vec![("key".repeat(1_000), Value::Bool(true)); 100],
			},
		}],
	};
	let bytes = wire::encode(&batch);
	assert!(
		bytes.len() < 4_000,
		"the dictionary stores the long key once"
	);
	let limits = wire::Limits {
		decoded_bytes: 100_000,
		..Default::default()
	};
	assert_eq!(
		wire::decode(&bytes, &limits).unwrap_err().reason,
		"decoded batch exceeds the memory limit"
	);
}

#[test]
fn unknown_entry_flags_are_refused() {
	// A valid header, one interned id, no contexts, and one entry with an
	// unassigned head bit. The decoder must reject the flag before its body.
	let bytes = b"LKE\x01\x00\x01\x01v\x00\x01\x40";
	assert_eq!(
		wire::decode(bytes, &wire::Limits::default())
			.unwrap_err()
			.reason,
		"unknown entry flags"
	);
}
