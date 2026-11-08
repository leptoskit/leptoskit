use leptoskit_bridge::Bytes;
use leptoskit_bridge::Plugin;

use super::*;

/// The smallest valid intent: `ACTION_VIEW` and nothing else. The Kotlin
/// suite decodes these same bytes.
const GOLDEN_MINIMAL: &[u8] = &[
	1, 0, 0, 0, // version
	0, 0, 0, 0, // flags
	26, 0, 0, 0, b'a', b'n', b'd', b'r', b'o', b'i', b'd', b'.', b'i', b'n', b't', b'e', b'n',
	b't', b'.', b'a', b'c', b't', b'i', b'o', b'n', b'.', b'V', b'I', b'E', b'W', // action
	0, 0, 0, 0, // data
	0, 0, 0, 0, // mime type
	0, 0, 0, 0, // package
	0, 0, 0, 0, // component
	0, 0, 0, 0, // categories
	0, 0, 0, 0, // extras
];

/// Every field and every extra tag, in hex. The Kotlin suite decodes these
/// same bytes and checks each decoded value.
const GOLDEN_FULL: &str = include_str!("../tests/golden_full.hex");

fn golden_full() -> Vec<u8> {
	let hex: String = GOLDEN_FULL.split_whitespace().collect();

	(0..hex.len())
		.step_by(2)
		.map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
		.collect()
}

fn full_intent() -> Intent {
	Intent::new()
		.action(action::SEND)
		.category(category::DEFAULT)
		.category(category::BROWSABLE)
		.data("https://example.com/a%20b?q=1")
		.mime_type("text/plain")
		.package("com.example.app")
		.component(".ShareActivity")
		.flag(flag::ACTIVITY_NEW_TASK)
		.flag(flag::ACTIVITY_CLEAR_TOP)
		.extra("bool", true)
		.extra("int", -7)
		.extra("long", 1_i64 << 40)
		.extra("double", 2.5)
		.extra("string", "héllo")
		.extra("empty", "")
		.extra(
			"bundle",
			Extras::new()
				.put("inner", 1)
				.put("deeper", Extras::new().put("flag", false)),
		)
		.extra("bytes", vec![0_u8, 255])
		.extra("bools", vec![true, false])
		.extra("ints", vec![1, -1])
		.extra("longs", vec![i64::MIN, i64::MAX])
		.extra("doubles", vec![-0.5, 1e300])
		.extra("strings", vec!["a", "β"])
		.extra("int_list", Extra::IntList(vec![3]))
		.extra("string_list", Extra::StringList(vec!["x".to_owned()]))
}

fn view() -> Intent {
	Intent::new().action(action::VIEW)
}

/// Encodes an intent's fields by hand, for payloads the builder refuses.
#[derive(Default)]
struct Raw {
	version: u32,
	flags: u32,
	action: Vec<u8>,
	data: Vec<u8>,
	mime: Vec<u8>,
	package: Vec<u8>,
	component: Vec<u8>,
	categories: Vec<Vec<u8>>,
	extras: Vec<u8>,
}

impl Raw {
	fn view() -> Self {
		Self {
			version: 1,
			action: action::VIEW.into(),
			extras: 0_u32.to_le_bytes().to_vec(),
			..Self::default()
		}
	}

	fn bytes(&self) -> Vec<u8> {
		let mut out = Vec::new();
		out.extend(self.version.to_le_bytes());
		out.extend(self.flags.to_le_bytes());

		for field in [
			&self.action,
			&self.data,
			&self.mime,
			&self.package,
			&self.component,
		] {
			string(&mut out, field);
		}

		out.extend((self.categories.len() as u32).to_le_bytes());

		for category in &self.categories {
			string(&mut out, category);
		}

		out.extend(&self.extras);
		out
	}
}

fn string(out: &mut Vec<u8>, value: &[u8]) {
	out.extend((value.len() as u32).to_le_bytes());
	out.extend(value);
}

/// One bundle of raw entries: key, tag, value bytes.
fn bundle(entries: &[(&[u8], u32, Vec<u8>)]) -> Vec<u8> {
	let mut out = (entries.len() as u32).to_le_bytes().to_vec();

	for (key, tag, value) in entries {
		string(&mut out, key);
		out.extend(tag.to_le_bytes());
		out.extend(value);
	}

	out
}

fn with_extras(extras: Vec<u8>) -> Vec<u8> {
	Raw {
		extras,
		..Raw::view()
	}
	.bytes()
}

#[test]
fn minimal_intent_matches_the_golden_bytes() {
	assert_eq!(view().encode().unwrap(), GOLDEN_MINIMAL);
	assert_eq!(Intent::decode(GOLDEN_MINIMAL).unwrap(), view());
}

#[test]
fn full_intent_matches_the_golden_bytes_and_round_trips() {
	let intent = full_intent();
	let bytes = intent.encode().unwrap();

	assert_eq!(bytes, golden_full());
	assert_eq!(Intent::decode(&bytes).unwrap(), intent);
	assert_eq!(intent.flags, 0x1000_0000 | 0x0400_0000);
	assert_eq!(intent.extras.len(), 15);
	assert_eq!(intent.extras.get("int"), Some(&Extra::Int(-7)));
	assert_eq!(intent.extras.get("missing"), None);
}

#[test]
fn every_extra_variant_round_trips_with_its_tag() {
	let values = [
		(Extra::Bool(false), 1),
		(Extra::Int(i32::MIN), 2),
		(Extra::Long(i64::MAX), 3),
		(Extra::Double(-0.0), 4),
		(Extra::String(String::new()), 5),
		(Extra::Bundle(Extras::new()), 6),
		(Extra::Bytes(vec![]), 7),
		(Extra::BoolArray(vec![true]), 8),
		(Extra::IntArray(vec![]), 9),
		(Extra::LongArray(vec![0]), 10),
		(Extra::DoubleArray(vec![f64::MAX]), 11),
		(Extra::StringArray(vec![String::new()]), 12),
		(Extra::IntList(vec![i32::MAX]), 13),
		(Extra::StringList(vec![]), 14),
	];

	for (value, tag) in values {
		assert_eq!(value.tag(), tag);
		let intent = view().extra("k", value);
		assert_eq!(
			Intent::decode(&intent.encode().unwrap()).unwrap(),
			intent,
			"tag {tag}"
		);
	}
}

#[test]
fn conversions_pick_the_matching_bundle_type() {
	assert_eq!(Extra::from(true), Extra::Bool(true));
	assert_eq!(Extra::from(1_i32), Extra::Int(1));
	assert_eq!(Extra::from(1_i64), Extra::Long(1));
	assert_eq!(Extra::from(1.0), Extra::Double(1.0));
	assert_eq!(Extra::from("a"), Extra::String("a".into()));
	assert_eq!(Extra::from(String::from("a")), Extra::String("a".into()));
	assert_eq!(Extra::from(&[1_u8][..]), Extra::Bytes(vec![1]));
	assert_eq!(Extra::from(vec![1_u8]), Extra::Bytes(vec![1]));
	assert_eq!(Extra::from(vec![true]), Extra::BoolArray(vec![true]));
	assert_eq!(Extra::from(vec![1_i32]), Extra::IntArray(vec![1]));
	assert_eq!(Extra::from(vec![1_i64]), Extra::LongArray(vec![1]));
	assert_eq!(Extra::from(vec![1.0]), Extra::DoubleArray(vec![1.0]));
	assert_eq!(Extra::from(vec!["a"]), Extra::StringArray(vec!["a".into()]));
	assert_eq!(
		Extra::from(vec![String::from("a")]),
		Extra::StringArray(vec!["a".into()])
	);
	assert_eq!(Extra::from(Extras::new()), Extra::Bundle(Extras::new()));
}

#[test]
fn extras_replace_duplicate_keys_like_a_bundle() {
	let extras = Extras::new().put("a", 1).put("b", 2).put("a", 3);

	assert_eq!(extras.len(), 2);
	assert!(!extras.is_empty());
	assert!(Extras::new().is_empty());
	assert_eq!(extras.get("a"), Some(&Extra::Int(3)));
	let keys: Vec<_> = extras.iter().map(|(key, _)| key).collect();
	assert_eq!(keys, ["a", "b"]);
	let intent = view().extras(extras.clone()).extra("b", 4);
	assert_eq!(intent.extras.get("b"), Some(&Extra::Int(4)));
	assert_eq!((&extras).into_iter().count(), 2);
}

#[test]
fn flags_are_single_bits() {
	assert_eq!(Flag::new(0x10).unwrap().bits(), 0x10);
	assert_eq!(Flag::new(1 << 31).unwrap().bits(), 1 << 31);

	for bits in [0, 3, 0x1000_0001, u32::MAX] {
		assert_eq!(Flag::new(bits), Err(Error::InvalidArgument), "{bits:#x}");
	}

	let combined = view()
		.flag(flag::ACTIVITY_NEW_TASK)
		.flag(flag::ACTIVITY_NEW_TASK)
		.flag(flag::ACTIVITY_NO_HISTORY);
	assert_eq!(combined.flags, 0x5000_0000);
}

#[test]
fn every_upstream_flag_except_the_grants_is_accepted() {
	let flags = [
		flag::ACTIVITY_BROUGHT_TO_FRONT,
		flag::ACTIVITY_CLEAR_TASK,
		flag::ACTIVITY_CLEAR_TOP,
		flag::ACTIVITY_CLEAR_WHEN_TASK_RESET,
		flag::ACTIVITY_EXCLUDE_FROM_RECENTS,
		flag::ACTIVITY_FORWARD_RESULT,
		flag::ACTIVITY_LAUNCHED_FROM_HISTORY,
		flag::ACTIVITY_LAUNCH_ADJACENT,
		flag::ACTIVITY_MATCH_EXTERNAL,
		flag::ACTIVITY_MULTIPLE_TASK,
		flag::ACTIVITY_NEW_DOCUMENT,
		flag::ACTIVITY_NEW_TASK,
		flag::ACTIVITY_NO_ANIMATION,
		flag::ACTIVITY_NO_HISTORY,
		flag::ACTIVITY_NO_USER_ACTION,
		flag::ACTIVITY_PREVIOUS_IS_TOP,
		flag::ACTIVITY_REORDER_TO_FRONT,
		flag::ACTIVITY_RESET_TASK_IF_NEEDED,
		flag::ACTIVITY_RETAIN_IN_RECENTS,
		flag::ACTIVITY_SINGLE_TOP,
		flag::ACTIVITY_TASK_ON_HOME,
		flag::DEBUG_LOG_RESOLUTION,
		flag::EXCLUDE_STOPPED_PACKAGES,
		flag::FROM_BACKGROUND,
		flag::INCLUDE_STOPPED_PACKAGES,
		flag::RECEIVER_FOREGROUND,
		flag::RECEIVER_NO_ABORT,
		flag::RECEIVER_REGISTERED_ONLY,
		flag::RECEIVER_REPLACE_PENDING,
		flag::RECEIVER_VISIBLE_TO_INSTANT_APPS,
	];

	for flag in flags {
		assert!(flag.bits().is_power_of_two());
		assert_eq!(flag.bits() & GRANT_URI_PERMISSION_FLAGS, 0);
		assert!(view().flag(flag).encode().is_ok(), "{:#x}", flag.bits());
	}

	let all = flags.iter().fold(view(), |intent, flag| intent.flag(*flag));
	assert!(all.encode().is_ok());
}

#[test]
fn uri_permission_grants_are_refused_on_both_sides() {
	for bits in [0x01, 0x02, 0x40, 0x80] {
		let intent = view().flag(Flag::new(bits).unwrap());
		assert_eq!(intent.encode(), Err(Error::PermissionDenied), "{bits:#x}");
		let raw = Raw {
			flags: bits | 0x1000_0000,
			..Raw::view()
		}
		.bytes();
		assert_eq!(
			Intent::decode(&raw),
			Err(Error::PermissionDenied),
			"{bits:#x}"
		);
	}

	// The policy check runs before anything after the flags is read.
	let mut truncated = Raw {
		flags: 0x01,
		..Raw::view()
	}
	.bytes();
	truncated.truncate(8);
	assert_eq!(Intent::decode(&truncated), Err(Error::PermissionDenied));
}

#[test]
fn file_uris_are_refused_and_other_schemes_are_checked() {
	for uri in [
		"file:///data/data/com.example/secret",
		"FILE:///sdcard/x",
		"File:x",
	] {
		assert_eq!(
			view().data(uri).encode(),
			Err(Error::PermissionDenied),
			"{uri}"
		);
	}

	for uri in [
		"https://example.com/",
		"content://com.example.provider/item/1",
		"package:com.example",
		"tel:+123",
		"geo:0,0?q=coffee",
		"market://details?id=com.example",
		"intent://x#Intent;end",
		"x-custom+v1.2:rest",
		"https://a/%41%7e",
	] {
		assert!(view().data(uri).encode().is_ok(), "{uri}");
	}

	for uri in [
		"no-scheme",
		":missing",
		"1http://a",
		"ht_tp://a",
		"https:",
		"https://a b",
		"https://a\\b",
		"https://a\n",
		"https://é",
		"https://a/%",
		"https://a/%4",
		"https://a/%zz",
		"https://a/%00",
		"https://a/%1F",
		"https://a/%7f",
	] {
		assert_eq!(
			view().data(uri).encode(),
			Err(Error::InvalidArgument),
			"{uri:?}"
		);
	}
}

#[test]
fn identifiers_follow_android_syntax() {
	for package in ["android", "com.example", "com.Example_1.app2"] {
		assert!(view().package(package).encode().is_ok(), "{package}");
	}

	for package in [
		".com", "com.", "com..app", "1com.app", "com.1app", "com.a-b", "com.é", "_com",
	] {
		assert_eq!(
			view().package(package).encode(),
			Err(Error::InvalidArgument),
			"{package}"
		);
	}

	for class in [
		".Main",
		"com.example.Main",
		"Main",
		"a.b.Outer$Inner",
		"_x.$y9",
	] {
		assert!(
			view()
				.package("com.example")
				.component(class)
				.encode()
				.is_ok(),
			"{class}"
		);
	}

	for class in [
		".",
		"..Main",
		"com..Main",
		"com.9Main",
		"com.Main.",
		"com.Ma-in",
		"com.Main ",
	] {
		assert_eq!(
			view().package("com.example").component(class).encode(),
			Err(Error::InvalidArgument),
			"{class}"
		);
	}

	for action in ["a b", "a\tb", "é", "a\u{7f}"] {
		assert_eq!(
			Intent::new().action(action).encode(),
			Err(Error::InvalidArgument),
			"{action:?}"
		);
	}

	for category in ["", "a b"] {
		assert_eq!(
			view().category(category).encode(),
			Err(Error::InvalidArgument),
			"{category:?}"
		);
	}
}

#[test]
fn mime_types_are_lowercase_type_and_subtype() {
	for mime in [
		"text/plain",
		"*/*",
		"image/*",
		"application/vnd.android.package-archive",
		"a+b/c.d",
	] {
		assert!(view().mime_type(mime).encode().is_ok(), "{mime}");
	}

	for mime in [
		"text",
		"/plain",
		"text/",
		"Text/plain",
		"text/plain; charset=utf-8",
		"a/b/c",
		"a b/c",
	] {
		assert_eq!(
			view().mime_type(mime).encode(),
			Err(Error::InvalidArgument),
			"{mime}"
		);
	}
}

#[test]
fn an_intent_needs_an_action_or_a_packaged_component() {
	assert_eq!(Intent::new().encode(), Err(Error::InvalidArgument));
	assert_eq!(
		Intent::new().package("com.example").encode(),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		Intent::new().component("com.example.Main").encode(),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		view().component("com.example.Main").encode(),
		Err(Error::InvalidArgument)
	);
	assert!(
		Intent::new()
			.package("com.example")
			.component(".Main")
			.encode()
			.is_ok()
	);
	assert!(view().package("com.example").encode().is_ok());
}

#[test]
fn empty_optional_values_are_refused_rather_than_dropped() {
	assert_eq!(
		Intent::new().action("").encode(),
		Err(Error::InvalidArgument)
	);
	assert_eq!(view().data("").encode(), Err(Error::InvalidArgument));
	assert_eq!(view().mime_type("").encode(), Err(Error::InvalidArgument));
	assert_eq!(view().package("").encode(), Err(Error::InvalidArgument));
	assert_eq!(
		view().package("com.example").component("").encode(),
		Err(Error::InvalidArgument)
	);
}

#[test]
fn categories_are_bounded_and_unique() {
	let full = (0..MAX_CATEGORIES).fold(view(), |intent, index| {
		intent.category(format!("c.{index}"))
	});
	assert!(full.encode().is_ok());
	assert_eq!(
		full.clone().category("c.extra").encode(),
		Err(Error::TooLarge)
	);
	assert_eq!(
		view()
			.category(category::DEFAULT)
			.category(category::DEFAULT)
			.encode(),
		Err(Error::InvalidArgument)
	);
}

#[test]
fn string_fields_are_bounded_exactly() {
	let name = |len: usize| "a".repeat(len);
	assert!(Intent::new().action(name(MAX_NAME_BYTES)).encode().is_ok());
	assert_eq!(
		Intent::new().action(name(MAX_NAME_BYTES + 1)).encode(),
		Err(Error::TooLarge)
	);
	assert!(view().category(name(MAX_NAME_BYTES)).encode().is_ok());
	assert_eq!(
		view().category(name(MAX_NAME_BYTES + 1)).encode(),
		Err(Error::TooLarge)
	);
	assert!(view().package(name(MAX_NAME_BYTES)).encode().is_ok());
	assert_eq!(
		view().package(name(MAX_NAME_BYTES + 1)).encode(),
		Err(Error::TooLarge)
	);
	let packaged = || view().package("com.example");
	assert!(packaged().component(name(MAX_NAME_BYTES)).encode().is_ok());
	assert_eq!(
		packaged().component(name(MAX_NAME_BYTES + 1)).encode(),
		Err(Error::TooLarge)
	);

	let mime = |len: usize| format!("a/{}", "b".repeat(len - 2));
	assert!(view().mime_type(mime(MAX_MIME_BYTES)).encode().is_ok());
	assert_eq!(
		view().mime_type(mime(MAX_MIME_BYTES + 1)).encode(),
		Err(Error::TooLarge)
	);

	let uri = |len: usize| format!("https://a/{}", "b".repeat(len - 10));
	assert!(view().data(uri(MAX_URI_BYTES)).encode().is_ok());
	assert_eq!(
		view().data(uri(MAX_URI_BYTES + 1)).encode(),
		Err(Error::TooLarge)
	);

	assert!(view().extra(name(MAX_NAME_BYTES), 1).encode().is_ok());
	assert_eq!(
		view().extra(name(MAX_NAME_BYTES + 1), 1).encode(),
		Err(Error::TooLarge)
	);
	assert!(view().extra("s", name(MAX_VALUE_BYTES)).encode().is_ok());
	assert_eq!(
		view().extra("s", name(MAX_VALUE_BYTES + 1)).encode(),
		Err(Error::TooLarge)
	);
	assert!(
		view()
			.extra("b", vec![0_u8; MAX_VALUE_BYTES])
			.encode()
			.is_ok()
	);
	assert_eq!(
		view().extra("b", vec![0_u8; MAX_VALUE_BYTES + 1]).encode(),
		Err(Error::TooLarge)
	);
	assert!(
		view()
			.extra("a", vec![name(MAX_VALUE_BYTES)])
			.encode()
			.is_ok()
	);
	assert_eq!(
		view().extra("a", vec![name(MAX_VALUE_BYTES + 1)]).encode(),
		Err(Error::TooLarge)
	);
}

#[test]
fn arrays_are_bounded_exactly() {
	let arrays: [fn(usize) -> Extra; 7] = [
		|len| Extra::BoolArray(vec![true; len]),
		|len| Extra::IntArray(vec![0; len]),
		|len| Extra::LongArray(vec![0; len]),
		|len| Extra::DoubleArray(vec![0.0; len]),
		|len| Extra::StringArray(vec![String::new(); len]),
		|len| Extra::IntList(vec![0; len]),
		|len| Extra::StringList(vec![String::new(); len]),
	];

	for array in arrays {
		assert!(view().extra("a", array(MAX_ARRAY_LEN)).encode().is_ok());
		assert_eq!(
			view().extra("a", array(MAX_ARRAY_LEN + 1)).encode(),
			Err(Error::TooLarge)
		);
	}
}

#[test]
fn bundles_are_bounded_in_width_depth_and_total() {
	let wide = |count: usize| {
		(0..count).fold(Extras::new(), |extras, index| {
			extras.put(format!("k{index}"), index as i32)
		})
	};
	assert!(view().extras(wide(MAX_EXTRAS)).encode().is_ok());
	assert_eq!(
		view().extras(wide(MAX_EXTRAS + 1)).encode(),
		Err(Error::TooLarge)
	);
	assert_eq!(
		view().extra("n", wide(MAX_EXTRAS + 1)).encode(),
		Err(Error::TooLarge)
	);

	// Depth 1 is the intent's own extras.
	let nested = |depth: usize| {
		(1..depth).fold(Extras::new().put("leaf", 1), |inner, _| {
			Extras::new().put("n", inner)
		})
	};
	assert!(view().extras(nested(MAX_DEPTH)).encode().is_ok());
	assert_eq!(
		view().extras(nested(MAX_DEPTH + 1)).encode(),
		Err(Error::TooLarge)
	);

	// Four bundles of 63 plus four parents is 256 entries.
	let total = |last: usize| {
		(0..4).fold(Extras::new(), |extras, index| {
			let width = if index == 3 { last } else { MAX_EXTRAS - 1 };
			extras.put(format!("b{index}"), wide(width))
		})
	};
	assert!(view().extras(total(MAX_EXTRAS - 1)).encode().is_ok());
	assert_eq!(
		view().extras(total(MAX_EXTRAS)).encode(),
		Err(Error::TooLarge)
	);
}

#[test]
fn the_whole_intent_is_bounded() {
	// The minimal intent is 62 bytes and each `kN` string extra adds 14
	// bytes of framing, so these two values fill the payload exactly.
	let first = "a".repeat(MAX_VALUE_BYTES);
	let exact = |second: usize| {
		view()
			.extra("k0", first.as_str())
			.extra("k1", "b".repeat(second))
	};
	let fill = MAX_INTENT_BYTES - GOLDEN_MINIMAL.len() - 2 * 14 - MAX_VALUE_BYTES;
	assert_eq!(exact(fill).encode().unwrap().len(), MAX_INTENT_BYTES);
	assert_eq!(exact(fill + 1).encode(), Err(Error::TooLarge));
	assert_eq!(
		Intent::decode(&vec![0; MAX_INTENT_BYTES + 1]),
		Err(Error::TooLarge)
	);
	assert_eq!(
		Intent::decode(&vec![0; MAX_INTENT_BYTES]),
		Err(Error::InvalidArgument)
	);
}

#[test]
fn every_truncation_and_trailing_byte_is_refused() {
	let bytes = full_intent().encode().unwrap();

	for len in 0..bytes.len() {
		assert_eq!(
			Intent::decode(&bytes[..len]),
			Err(Error::InvalidArgument),
			"prefix {len}"
		);
	}

	let mut trailing = bytes.clone();
	trailing.push(0);
	assert_eq!(Intent::decode(&trailing), Err(Error::InvalidArgument));
}

#[test]
fn malformed_headers_are_refused() {
	for version in [0, 2, u32::MAX] {
		assert_eq!(
			Intent::decode(
				&Raw {
					version,
					..Raw::view()
				}
				.bytes()
			),
			Err(Error::InvalidArgument)
		);
	}

	let bad_utf8 = Raw {
		action: vec![0xff],
		..Raw::view()
	}
	.bytes();
	assert_eq!(Intent::decode(&bad_utf8), Err(Error::InvalidArgument));

	// A declared length past its bound is too large even when truncated.
	let mut oversized = 1_u32.to_le_bytes().to_vec();
	oversized.extend(0_u32.to_le_bytes());
	oversized.extend(((MAX_NAME_BYTES + 1) as u32).to_le_bytes());
	assert_eq!(Intent::decode(&oversized), Err(Error::TooLarge));

	let categories = Raw {
		categories: vec![vec![0xc3]],
		..Raw::view()
	}
	.bytes();
	assert_eq!(Intent::decode(&categories), Err(Error::InvalidArgument));
}

#[test]
fn malformed_extras_are_refused() {
	let cases: Vec<(&str, Vec<u8>, Error)> = vec![
		(
			"unknown tag",
			bundle(&[(b"k", 15, vec![])]),
			Error::InvalidArgument,
		),
		(
			"tag zero",
			bundle(&[(b"k", 0, vec![])]),
			Error::InvalidArgument,
		),
		(
			"bool word 2",
			bundle(&[(b"k", 1, 2_u32.to_le_bytes().to_vec())]),
			Error::InvalidArgument,
		),
		(
			"bool array word 2",
			bundle(&[(b"k", 8, [1_u32.to_le_bytes(), 2_u32.to_le_bytes()].concat())]),
			Error::InvalidArgument,
		),
		(
			"NaN",
			bundle(&[(b"k", 4, f64::NAN.to_bits().to_le_bytes().to_vec())]),
			Error::InvalidArgument,
		),
		(
			"infinity",
			bundle(&[(b"k", 4, f64::INFINITY.to_bits().to_le_bytes().to_vec())]),
			Error::InvalidArgument,
		),
		(
			"NaN element",
			bundle(&[(
				b"k",
				11,
				[
					1_u32.to_le_bytes().to_vec(),
					f64::NAN.to_bits().to_le_bytes().to_vec(),
				]
				.concat(),
			)]),
			Error::InvalidArgument,
		),
		(
			"empty key",
			bundle(&[(b"", 2, vec![0; 4])]),
			Error::InvalidArgument,
		),
		(
			"control key",
			bundle(&[(b"a\nb", 2, vec![0; 4])]),
			Error::InvalidArgument,
		),
		(
			"C1 control key",
			bundle(&[("a\u{85}".as_bytes(), 2, vec![0; 4])]),
			Error::InvalidArgument,
		),
		(
			"invalid key",
			bundle(&[(&[0xff], 2, vec![0; 4])]),
			Error::InvalidArgument,
		),
		(
			"duplicate key",
			bundle(&[(b"k", 2, vec![0; 4]), (b"k", 2, vec![0; 4])]),
			Error::InvalidArgument,
		),
		(
			"invalid string",
			bundle(&[(b"k", 5, [1_u32.to_le_bytes().to_vec(), vec![0xff]].concat())]),
			Error::InvalidArgument,
		),
		(
			"oversized count",
			((MAX_EXTRAS + 1) as u32).to_le_bytes().to_vec(),
			Error::TooLarge,
		),
		(
			"oversized array",
			bundle(&[(b"k", 9, ((MAX_ARRAY_LEN + 1) as u32).to_le_bytes().to_vec())]),
			Error::TooLarge,
		),
		(
			"oversized string",
			bundle(&[(
				b"k",
				5,
				((MAX_VALUE_BYTES + 1) as u32).to_le_bytes().to_vec(),
			)]),
			Error::TooLarge,
		),
	];

	for (name, extras, error) in cases {
		assert_eq!(Intent::decode(&with_extras(extras)), Err(error), "{name}");
	}

	// Duplicate keys in different bundles are fine.
	let nested = bundle(&[(b"k", 6, bundle(&[(b"k", 2, vec![0; 4])]))]);
	assert!(Intent::decode(&with_extras(nested)).is_ok());

	// Non-finite doubles are refused by the encoder too.
	assert_eq!(
		view().extra("d", f64::NAN).encode(),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		view().extra("d", vec![f64::NEG_INFINITY]).encode(),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		view().extra("k\u{0}", 1).encode(),
		Err(Error::InvalidArgument)
	);
	assert_eq!(view().extra("", 1).encode(), Err(Error::InvalidArgument));
}

#[test]
fn choosers_carry_a_bounded_plain_title() {
	let intent = view();

	for title in ["", "Open with", "选择"] {
		let bytes = Chooser::encode(&intent, title).unwrap();
		let chooser = Chooser::decode(&bytes).unwrap();
		assert_eq!(chooser.title, title);
		assert_eq!(chooser.intent, intent);
	}

	let bytes = Chooser::encode(&intent, "Go").unwrap();
	assert_eq!(&bytes[..6], &[2, 0, 0, 0, b'G', b'o']);
	assert_eq!(&bytes[6..], GOLDEN_MINIMAL);

	assert!(Chooser::encode(&intent, &"t".repeat(MAX_TITLE_BYTES)).is_ok());
	assert_eq!(
		Chooser::encode(&intent, &"t".repeat(MAX_TITLE_BYTES + 1)),
		Err(Error::TooLarge)
	);
	assert_eq!(
		Chooser::encode(&intent, "a\nb"),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		Chooser::encode(&Intent::new(), "t"),
		Err(Error::InvalidArgument)
	);

	let mut oversized = ((MAX_TITLE_BYTES + 1) as u32).to_le_bytes().to_vec();
	oversized.extend(GOLDEN_MINIMAL);
	assert_eq!(Chooser::decode(&oversized), Err(Error::TooLarge));
	assert_eq!(
		Chooser::decode(&[1, 0, 0, 0, 0xff]),
		Err(Error::InvalidArgument)
	);
	assert_eq!(Chooser::decode(&[0, 0, 0]), Err(Error::InvalidArgument));
	assert_eq!(Chooser::decode(&[0, 0, 0, 0]), Err(Error::InvalidArgument));
}

#[test]
fn intent_uris_must_be_complete_intent_scheme_uris() {
	for uri in [
		"intent:#Intent;action=android.settings.SETTINGS;end",
		"intent://scan/#Intent;scheme=zxing;package=com.google.zxing.client.android;end",
		"intent://a/#Intent;S.browser_fallback_url=https%3A%2F%2Fexample.com;end",
	] {
		let parsed = IntentUri::parse(uri).unwrap();
		assert_eq!(parsed.as_str(), uri);
		assert_eq!(IntentUri::decode(uri.as_bytes()), Ok(parsed));
	}

	for uri in [
		"",
		"https://example.com",
		"INTENT:#Intent;end",
		"intent://a",
		"intent://a#Intent;",
		"intent://a #Intent;end",
		"intent://a/%00#Intent;end",
		"intent://a/#Intent;end\n",
	] {
		assert_eq!(
			IntentUri::parse(uri),
			Err(Error::InvalidArgument),
			"{uri:?}"
		);
	}

	let long = format!("intent://{}#Intent;end", "a".repeat(MAX_URI_BYTES));
	assert_eq!(IntentUri::parse(&long), Err(Error::TooLarge));
	assert_eq!(IntentUri::decode(long.as_bytes()), Err(Error::TooLarge));
	let exact = format!("intent://{}#Intent;end", "a".repeat(MAX_URI_BYTES - 20));
	assert_eq!(exact.len(), MAX_URI_BYTES);
	assert!(IntentUri::parse(&exact).is_ok());
	assert_eq!(IntentUri::decode(&[0xff]), Err(Error::InvalidArgument));
}

#[test]
fn boolean_replies_are_exact_words() {
	assert_eq!(encode_bool(true), [1, 0, 0, 0]);
	assert_eq!(encode_bool(false), [0, 0, 0, 0]);
	assert_eq!(decode_bool(&[1, 0, 0, 0]), Ok(true));
	assert_eq!(decode_bool(&[0, 0, 0, 0]), Ok(false));

	for reply in [
		&[][..],
		&[1],
		&[2, 0, 0, 0],
		&[0, 0, 0, 1],
		&[1, 0, 0, 0, 0],
	] {
		assert_eq!(decode_bool(reply), Err(Error::InvalidArgument), "{reply:?}");
	}
}

#[test]
fn resolved_activity_replies_are_strict() {
	let settings = ResolvedActivity {
		package_name: "com.android.settings".into(),
		activity_name: "com.android.settings.Settings".into(),
		label: "Settings".into(),
	};
	let bytes = ResolvedActivity::encode(Some(&settings)).unwrap();
	let mut expected = vec![1, 0, 0, 0];
	string(&mut expected, b"com.android.settings");
	string(&mut expected, b"com.android.settings.Settings");
	string(&mut expected, b"Settings");
	assert_eq!(bytes, expected);
	assert_eq!(ResolvedActivity::decode(&bytes), Ok(Some(settings.clone())));
	assert_eq!(ResolvedActivity::encode(None).unwrap(), [0, 0, 0, 0]);
	assert_eq!(ResolvedActivity::decode(&[0, 0, 0, 0]), Ok(None));

	let empty_label = ResolvedActivity {
		label: String::new(),
		..settings.clone()
	};
	assert!(ResolvedActivity::encode(Some(&empty_label)).is_ok());
	let long_label = ResolvedActivity {
		label: "l".repeat(MAX_LABEL_BYTES),
		..settings.clone()
	};
	assert!(ResolvedActivity::encode(Some(&long_label)).is_ok());
	let too_long = ResolvedActivity {
		label: "l".repeat(MAX_LABEL_BYTES + 1),
		..settings.clone()
	};
	assert_eq!(
		ResolvedActivity::encode(Some(&too_long)),
		Err(Error::TooLarge)
	);
	let relative = ResolvedActivity {
		activity_name: ".Settings".into(),
		..settings.clone()
	};
	assert_eq!(
		ResolvedActivity::encode(Some(&relative)),
		Err(Error::InvalidArgument)
	);
	let bad_package = ResolvedActivity {
		package_name: "com..settings".into(),
		..settings
	};
	assert_eq!(
		ResolvedActivity::encode(Some(&bad_package)),
		Err(Error::InvalidArgument)
	);

	for reply in [&[][..], &[0, 0, 0, 0, 0], &[2, 0, 0, 0], &[1, 0, 0, 0]] {
		assert_eq!(
			ResolvedActivity::decode(reply),
			Err(Error::InvalidArgument),
			"{reply:?}"
		);
	}

	for len in 4..bytes.len() {
		assert_eq!(
			ResolvedActivity::decode(&bytes[..len]),
			Err(Error::InvalidArgument),
			"prefix {len}"
		);
	}

	let mut trailing = bytes;
	trailing.push(0);
	assert_eq!(
		ResolvedActivity::decode(&trailing),
		Err(Error::InvalidArgument)
	);
}

#[test]
fn requests_decode_per_method() {
	let intent = GOLDEN_MINIMAL;
	assert_eq!(decode_request(LAUNCH, intent), Ok(Request::Launch(view())));
	assert_eq!(
		decode_request(SEND_BROADCAST, intent),
		Ok(Request::SendBroadcast(view()))
	);
	assert_eq!(
		decode_request(CAN_RESOLVE_ACTIVITY, intent),
		Ok(Request::CanResolveActivity(view()))
	);
	assert_eq!(
		decode_request(RESOLVED_ACTIVITY, intent),
		Ok(Request::ResolvedActivity(view()))
	);
	let chooser = Chooser::encode(&view(), "t").unwrap();
	assert!(matches!(
		decode_request(LAUNCH_CHOOSER, &chooser),
		Ok(Request::LaunchChooser(_))
	));
	let uri = b"intent:#Intent;action=a;end";
	assert!(matches!(
		decode_request(PARSE_AND_LAUNCH, uri),
		Ok(Request::ParseAndLaunch(_))
	));

	// Android refuses implicit service intents.
	assert_eq!(
		decode_request(SEND_SERVICE, intent),
		Err(Error::InvalidArgument)
	);
	let service = view().package("com.example").encode().unwrap();
	assert!(matches!(
		decode_request(SEND_SERVICE, &service),
		Ok(Request::SendService(_))
	));

	assert_eq!(decode_request(MethodId(7), intent), Err(Error::Unavailable));
	assert_eq!(decode_request(LAUNCH, &[]), Err(Error::InvalidArgument));
	assert_eq!(
		decode_request(PARSE_AND_LAUNCH, intent),
		Err(Error::InvalidArgument)
	);
	assert_eq!(METHODS.len(), 7);
}

#[test]
fn unsupported_hosts_validate_then_report_unavailable() {
	let mut plugin = Unsupported;
	assert_eq!(plugin.name(), NAME);
	assert_eq!(plugin.methods(), METHODS);
	let intent = Bytes::from_static(GOLDEN_MINIMAL);

	for method in [
		LAUNCH,
		SEND_BROADCAST,
		CAN_RESOLVE_ACTIVITY,
		RESOLVED_ACTIVITY,
	] {
		assert_eq!(
			plugin.invoke(method, intent.clone()),
			Err(Error::Unavailable)
		);
		assert_eq!(
			plugin.invoke(method, Bytes::new()),
			Err(Error::InvalidArgument)
		);
	}

	let granted = Raw {
		flags: 1,
		..Raw::view()
	}
	.bytes();
	assert_eq!(
		plugin.invoke(LAUNCH, granted.into()),
		Err(Error::PermissionDenied)
	);
	assert_eq!(
		plugin.invoke(SEND_SERVICE, intent),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		plugin.invoke(PARSE_AND_LAUNCH, Bytes::from_static(b"intent:#Intent;end")),
		Err(Error::Unavailable)
	);
	assert_eq!(
		plugin.invoke(MethodId(99), Bytes::new()),
		Err(Error::Unavailable)
	);
}

#[test]
fn kotlin_suite_decodes_the_same_golden_bytes() {
	let kotlin = include_str!("../tests/android/AndroidIntentWireTest.kt");

	for line in GOLDEN_FULL.lines() {
		assert!(
			kotlin.contains(&format!("\"{line}\"")),
			"Kotlin golden bytes drifted at {line}"
		);
	}

	let kotlin_wire = include_str!("../android/IntentWire.kt");

	for (name, value) in [
		("MAX_INTENT_BYTES", MAX_INTENT_BYTES),
		("MAX_NAME_BYTES", MAX_NAME_BYTES),
		("MAX_MIME_BYTES", MAX_MIME_BYTES),
		("MAX_URI_BYTES", MAX_URI_BYTES),
		("MAX_VALUE_BYTES", MAX_VALUE_BYTES),
		("MAX_ARRAY_LEN", MAX_ARRAY_LEN),
		("MAX_EXTRAS", MAX_EXTRAS),
		("MAX_TOTAL_EXTRAS", MAX_TOTAL_EXTRAS),
		("MAX_DEPTH", MAX_DEPTH),
		("MAX_CATEGORIES", MAX_CATEGORIES),
		("MAX_TITLE_BYTES", MAX_TITLE_BYTES),
		("MAX_LABEL_BYTES", MAX_LABEL_BYTES),
	] {
		let declaration = kotlin_wire
			.lines()
			.find_map(|line| line.trim().strip_prefix(&format!("const val {name} = ")))
			.unwrap_or_else(|| panic!("Kotlin lacks {name}"));
		let kotlin_value: usize = declaration
			.split(" * ")
			.map(|factor| factor.trim().parse::<usize>().unwrap())
			.product();
		assert_eq!(kotlin_value, value, "{name}");
	}
}
