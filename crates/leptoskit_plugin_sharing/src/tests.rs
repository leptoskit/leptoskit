use super::*;

/// The request the Swift and Kotlin suites decode byte for byte: text "hi",
/// subject "S", title "T", anchor (1, 2, 3, 4), Mail excluded, an app file
/// `a.txt` as `text/plain`, and the bytes 1, 2, 3 named `b.bin`.
const GOLDEN_REQUEST: &[u8] = &[
	1, 0, 0, 0, // version
	2, 0, 0, 0, b'h', b'i', // text
	0, 0, 0, 0, // no URL
	1, 0, 0, 0, b'S', // subject
	1, 0, 0, 0, b'T', // title
	1, 0, 0, 0, // anchor present
	0, 0, 0x80, 0x3F, // 1.0
	0, 0, 0, 0x40, // 2.0
	0, 0, 0x40, 0x40, // 3.0
	0, 0, 0x80, 0x40, // 4.0
	1, 0, 0, 0, 4, 0, 0, 0, // Mail excluded
	2, 0, 0, 0, // two files
	0, 0, 0, 0, 5, 0, 0, 0, b'a', b'.', b't', b'x', b't', // app path
	10, 0, 0, 0, b't', b'e', b'x', b't', b'/', b'p', b'l', b'a', b'i', b'n', // MIME
	0, 0, 0, 0, // no name
	1, 0, 0, 0, 3, 0, 0, 0, 1, 2, 3, // bytes
	0, 0, 0, 0, // no MIME
	5, 0, 0, 0, b'b', b'.', b'b', b'i', b'n', // name
];

/// Replies the Swift and Kotlin suites encode byte for byte.
const GOLDEN_SUCCESS: &[u8] = &[1, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, b'x', b'.', b'y'];
const GOLDEN_DISMISSED: &[u8] = &[1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
const GOLDEN_UNAVAILABLE: &[u8] = &[1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0];

fn golden() -> Content {
	Content::files([
		File::app("a.txt").unwrap().with_mime("text/plain").unwrap(),
		File::bytes("b.bin", [1, 2, 3]).unwrap(),
	])
	.unwrap()
	.with_text("hi")
	.unwrap()
	.with_subject("S")
	.unwrap()
	.with_title("T")
	.unwrap()
	.with_anchor(Anchor::new(1.0, 2.0, 3.0, 4.0).unwrap())
	.excluding(AppleActivity::Mail)
}

fn round_trip(content: &Content) -> Bytes {
	let bytes = content.encode().unwrap();
	assert_eq!(&Content::decode(&bytes).unwrap(), content);

	for end in 0..bytes.len() {
		assert!(Content::decode(&bytes[..end]).is_err(), "prefix {end}");
	}

	let mut trailing = bytes.to_vec();
	trailing.push(0);
	assert!(Content::decode(&trailing).is_err());

	bytes
}

#[test]
fn golden_request_matches_the_native_suites() {
	let content = golden();
	assert_eq!(&*round_trip(&content), GOLDEN_REQUEST);
	assert_eq!(content.encoded_len(), GOLDEN_REQUEST.len());
	let decoded = Content::decode(GOLDEN_REQUEST).unwrap();
	assert_eq!(decoded.shared_text(), Some("hi"));
	assert_eq!(decoded.subject(), Some("S"));
	assert_eq!(decoded.title(), Some("T"));
	assert_eq!(decoded.shared_url(), None);
	assert_eq!(decoded.excluded(), [AppleActivity::Mail]);
	let anchor = decoded.anchor().unwrap();
	assert_eq!(
		[anchor.x(), anchor.y(), anchor.width(), anchor.height()],
		[1.0, 2.0, 3.0, 4.0]
	);
	let [app, bytes] = decoded.shared_files() else {
		panic!("two files expected");
	};
	assert_eq!(app.source(), &FileSource::App("a.txt".into()));
	assert_eq!((app.mime(), app.name()), (Some("text/plain"), None));
	assert_eq!(bytes.source(), &FileSource::Bytes(vec![1, 2, 3]));
	assert_eq!((bytes.mime(), bytes.name()), (None, Some("b.bin")));
}

#[test]
fn golden_replies_match_the_native_suites() {
	for (result, bytes) in [
		(ShareResult::Success("x.y".into()), GOLDEN_SUCCESS),
		(ShareResult::Dismissed, GOLDEN_DISMISSED),
		(ShareResult::Unavailable, GOLDEN_UNAVAILABLE),
	] {
		assert_eq!(&*result.encode().unwrap(), bytes);
		assert_eq!(ShareResult::decode(bytes).unwrap(), result);

		for end in 0..bytes.len() {
			assert!(ShareResult::decode(&bytes[..end]).is_err());
		}

		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert!(ShareResult::decode(&trailing).is_err());
	}
	assert_eq!(ShareResult::Success("x.y".into()).raw(), Some("x.y"));
	assert_eq!(ShareResult::Dismissed.raw(), None);
}

#[test]
fn replies_reject_unknown_versions_statuses_and_mismatched_raw_values() {
	let reply = |version: u32, status: u32, raw: &[u8]| {
		let mut encoder = Encoder::default();
		encoder.u32(version);
		encoder.u32(status);
		encoder.bytes(raw).unwrap();
		ShareResult::decode(&encoder.finish())
	};
	// The empty reply of the previous protocol is no longer a result.
	assert!(ShareResult::decode(&[]).is_err());
	assert_eq!(reply(0, 2, b""), Err(Error::InvalidArgument));
	assert_eq!(reply(2, 2, b""), Err(Error::InvalidArgument));
	assert_eq!(reply(1, 3, b""), Err(Error::InvalidArgument));
	assert_eq!(reply(1, 0, b""), Err(Error::InvalidArgument));
	assert_eq!(reply(1, 1, b"x"), Err(Error::InvalidArgument));
	assert_eq!(reply(1, 2, b"x"), Err(Error::InvalidArgument));
	assert_eq!(reply(1, 0, b"a\nb"), Err(Error::InvalidArgument));
	assert_eq!(reply(1, 0, &[0xFF]), Err(Error::InvalidArgument));
	assert_eq!(
		reply(1, 0, "a".repeat(MAX_RAW_BYTES + 1).as_bytes()),
		Err(Error::TooLarge)
	);
	let longest = "a".repeat(MAX_RAW_BYTES);
	assert_eq!(
		reply(1, 0, longest.as_bytes()),
		Ok(ShareResult::Success(longest))
	);
	assert_eq!(
		ShareResult::Success(String::new()).encode(),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		ShareResult::Success("a".repeat(MAX_RAW_BYTES + 1)).encode(),
		Err(Error::TooLarge)
	);
}

#[test]
fn text_and_url_bounds_and_framing() {
	for content in [
		Content::text("hello\n世界").unwrap(),
		Content::url("https://example.test/a%20b?q=one#two").unwrap(),
		Content::url("https://example.test")
			.unwrap()
			.with_subject("Subject")
			.unwrap()
			.with_title("Title")
			.unwrap(),
	] {
		round_trip(&content);
	}
	assert_eq!(Content::text(""), Err(Error::InvalidArgument));
	assert_eq!(Content::text("a\0b"), Err(Error::InvalidArgument));
	assert_eq!(
		Content::text("a".repeat(MAX_TEXT_BYTES + 1)),
		Err(Error::TooLarge)
	);
	assert!(Content::text("a".repeat(MAX_TEXT_BYTES)).is_ok());
	let url = Content::url("http://example.test:8080/path").unwrap();
	assert_eq!(url.shared_url(), Some("http://example.test:8080/path"));
	assert_eq!(url.shared_text(), None);
	let text = Content::text("https://not-validated-as-a-url").unwrap();
	assert_eq!(text.shared_url(), None);
	assert_eq!(text.shared_text(), Some("https://not-validated-as-a-url"));
}

#[test]
fn urls_do_not_grant_files_or_choose_recipients() {
	for value in [
		"file:///secret",
		"content://provider/item",
		"mailto:recipient@example.test",
		"javascript:alert(1)",
		"https://user:password@example.test",
		"https:///no-host",
		"https://example.test/a b",
		"https://example.test/%no",
		"https://example.test\\evil",
		"https://",
		// Passes the character checks but is not a parseable URL.
		"https://[::1/path",
	] {
		assert_eq!(Content::url(value), Err(Error::InvalidArgument), "{value}");
	}
	let long = format!("https://example.test/{}", "a".repeat(MAX_URL_BYTES));
	assert_eq!(Content::url(long), Err(Error::TooLarge));
}

#[test]
fn subject_and_title_are_bounded_single_line_labels() {
	let text = Content::text("body").unwrap();

	for (value, error) in [
		(String::new(), Error::InvalidArgument),
		("a\nb".into(), Error::InvalidArgument),
		("a\0b".into(), Error::InvalidArgument),
		("a".repeat(MAX_SUBJECT_BYTES + 1), Error::TooLarge),
	] {
		assert_eq!(text.clone().with_subject(value.clone()), Err(error));
		assert_eq!(text.clone().with_title(value), Err(error));
	}
	assert!(text.clone().with_subject("é".repeat(512)).is_ok());
	assert!(text.with_title("a".repeat(MAX_TITLE_BYTES)).is_ok());
}

#[test]
fn a_url_never_travels_with_text_or_files() {
	let url = Content::url("https://example.test").unwrap();
	assert_eq!(url.with_text("caption"), Err(Error::InvalidArgument));

	let mut encoder = Encoder::default();
	encoder.u32(WIRE_VERSION);
	encoder.bytes(b"").unwrap();
	encoder.bytes(b"https://example.test").unwrap();
	encoder.bytes(b"").unwrap();
	encoder.bytes(b"").unwrap();
	encoder.u32(0);
	encoder.u32(0);
	encoder.u32(1);
	encoder.u32(0);
	encoder.bytes(b"a.txt").unwrap();
	encoder.bytes(b"").unwrap();
	encoder.bytes(b"").unwrap();
	assert_eq!(
		Content::decode(&encoder.finish()),
		Err(Error::InvalidArgument)
	);

	let mut with_text = golden().encode().unwrap().to_vec();
	// Splice a URL into the golden request, which already carries text.
	with_text.splice(10..14, [14, 0, 0, 0].into_iter().chain(*b"https://e.test"));
	assert_eq!(Content::decode(&with_text), Err(Error::InvalidArgument));
}

#[test]
fn requests_reject_nothing_to_share_and_old_or_unknown_versions() {
	let mut empty = Encoder::default();
	empty.u32(WIRE_VERSION);

	for _ in 0..4 {
		empty.bytes(b"").unwrap();
	}

	empty.u32(0);
	empty.u32(0);
	empty.u32(0);
	assert_eq!(
		Content::decode(&empty.finish()),
		Err(Error::InvalidArgument)
	);

	// The previous protocol's text request: kind 0, then "x".
	assert!(Content::decode(&[0, 0, 0, 0, 1, 0, 0, 0, b'x']).is_err());
	let mut future = GOLDEN_REQUEST.to_vec();
	future[0] = 2;
	assert_eq!(Content::decode(&future), Err(Error::InvalidArgument));
	assert_eq!(
		Content::decode(&vec![0; MAX_REQUEST_BYTES + 1]),
		Err(Error::TooLarge)
	);
}

#[test]
fn anchors_are_finite_non_negative_and_bounded() {
	for values in [
		[f32::NAN, 0.0, 1.0, 1.0],
		[0.0, f32::INFINITY, 1.0, 1.0],
		[-1.0, 0.0, 1.0, 1.0],
		[0.0, 0.0, -0.5, 1.0],
		[0.0, 0.0, 1.0, MAX_ANCHOR_COORDINATE * 2.0],
	] {
		let [x, y, width, height] = values;
		assert_eq!(
			Anchor::new(x, y, width, height),
			Err(Error::InvalidArgument),
			"{values:?}"
		);
	}
	assert!(Anchor::new(0.0, 0.0, 0.0, 0.0).is_ok());
	assert!(Anchor::new(MAX_ANCHOR_COORDINATE, 0.0, 0.0, MAX_ANCHOR_COORDINATE).is_ok());

	let mut bytes = GOLDEN_REQUEST.to_vec();
	// Overwrite x with NaN, then use an unknown anchor flag.
	bytes[28..32].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
	assert_eq!(Content::decode(&bytes), Err(Error::InvalidArgument));
	let mut flag = GOLDEN_REQUEST.to_vec();
	flag[24] = 2;
	assert_eq!(Content::decode(&flag), Err(Error::InvalidArgument));
}

#[test]
fn excluded_activities_are_known_and_unique() {
	for activity in AppleActivity::ALL {
		assert_eq!(AppleActivity::from_code(activity.code()), Some(activity));
	}
	assert_eq!(AppleActivity::from_code(20), None);
	assert_eq!(AppleActivity::from_code(u32::MAX), None);

	let all = AppleActivity::ALL
		.into_iter()
		.fold(Content::text("t").unwrap(), Content::excluding)
		.excluding(AppleActivity::Mail);
	assert_eq!(all.excluded(), AppleActivity::ALL);
	round_trip(&all);

	let mut unknown = GOLDEN_REQUEST.to_vec();
	unknown[48] = 20;
	assert_eq!(Content::decode(&unknown), Err(Error::InvalidArgument));

	let encode = |codes: &[u32]| {
		let mut encoder = Encoder::default();
		encoder.u32(WIRE_VERSION);
		encoder.bytes(b"t").unwrap();

		for _ in 0..3 {
			encoder.bytes(b"").unwrap();
		}

		encoder.u32(0);
		encoder.u32(u32::try_from(codes.len()).unwrap());

		for code in codes {
			encoder.u32(*code);
		}

		encoder.u32(0);
		Content::decode(&encoder.finish())
	};
	assert_eq!(encode(&[4, 4]), Err(Error::InvalidArgument));
	assert_eq!(encode(&[0; ACTIVITY_COUNT + 1]), Err(Error::TooLarge));
	assert!(encode(&[19, 0]).is_ok());
}

#[test]
fn app_paths_stay_inside_the_files_directory() {
	for path in [
		"",
		"/etc/passwd",
		"a/../b",
		"../secret",
		"a/./b",
		"a//b",
		"a/",
		"C:hello",
		"a\\b",
		"\0",
		"con",
		"NUL.txt",
		"a/LPT9",
		"name.",
		"name ",
		"a?",
		".leptoskit-tmp-write",
		"dir/.leptoskit-tmp-quota-lock",
	] {
		assert_eq!(File::app(path), Err(Error::InvalidArgument), "{path:?}");
	}
	assert_eq!(
		File::app("a".repeat(MAX_PATH_BYTES + 1)),
		Err(Error::TooLarge)
	);
	assert!(File::app("a".repeat(MAX_PATH_BYTES)).is_ok());
	assert!(File::app("exports/report 2.pdf").is_ok());
	assert!(File::app(".hidden").is_ok());
}

#[test]
fn names_and_mime_types_are_strict() {
	for name in ["", "a/b", "..", ".", "a\\b", "a:b", "x\n", "trail."] {
		assert_eq!(
			File::bytes(name, *b"x"),
			Err(Error::InvalidArgument),
			"{name:?}"
		);
		assert_eq!(
			File::app("a").unwrap().with_name(name),
			Err(Error::InvalidArgument)
		);
	}
	assert_eq!(
		File::bytes("a".repeat(MAX_FILE_NAME_BYTES + 1), *b"x"),
		Err(Error::TooLarge)
	);
	assert!(File::bytes("a".repeat(MAX_FILE_NAME_BYTES), *b"x").is_ok());
	assert!(File::bytes("empty.txt", Vec::new()).is_ok());

	for mime in [
		"image/png",
		"image/*",
		"*/*",
		"application/vnd.openxmlformats-officedocument.wordprocessingml.document",
		"text/x-c++",
	] {
		assert_eq!(validate_mime(mime), Ok(()), "{mime}");
	}

	for mime in [
		"",
		"image",
		"image/",
		"/png",
		"*/png",
		"image/png; charset=utf-8",
		"image/ png",
		"image/png/x",
		"-image/png",
		"image/*x",
	] {
		assert_eq!(validate_mime(mime), Err(Error::InvalidArgument), "{mime}");
	}
	assert_eq!(
		validate_mime(&format!("a/{}", "b".repeat(MAX_MIME_BYTES))),
		Err(Error::TooLarge)
	);
}

#[test]
fn file_counts_and_inline_bytes_are_bounded() {
	let file = || File::app("a").unwrap();
	assert_eq!(Content::files([]), Err(Error::InvalidArgument));
	assert_eq!(
		Content::files(std::iter::repeat_with(file).take(MAX_FILES + 1)),
		Err(Error::TooLarge)
	);
	let most = Content::files(std::iter::repeat_with(file).take(MAX_FILES)).unwrap();
	round_trip(&most);

	assert_eq!(
		File::bytes("big", vec![0; MAX_INLINE_BYTES + 1]),
		Err(Error::TooLarge)
	);
	let half = || File::bytes("half", vec![0; MAX_INLINE_BYTES / 2]).unwrap();
	assert!(Content::files([half(), half()]).is_ok());
	assert_eq!(
		Content::files([half(), half(), File::bytes("one", [0]).unwrap()]),
		Err(Error::TooLarge)
	);

	// The largest possible request still fits the documented bound.
	let largest = Content::files((0..MAX_FILES).map(|index| {
		let file = if index == 0 {
			File::bytes("n".repeat(MAX_FILE_NAME_BYTES), vec![0; MAX_INLINE_BYTES]).unwrap()
		} else {
			File::app("p".repeat(MAX_PATH_BYTES))
				.unwrap()
				.with_name("n".repeat(MAX_FILE_NAME_BYTES))
				.unwrap()
		};
		file.with_mime(format!("{}/{}", "a".repeat(127), "b".repeat(127)))
			.unwrap()
	}))
	.unwrap()
	.with_text("t".repeat(MAX_TEXT_BYTES))
	.unwrap()
	.with_subject("s".repeat(MAX_SUBJECT_BYTES))
	.unwrap()
	.with_title("t".repeat(MAX_TITLE_BYTES))
	.unwrap()
	.with_anchor(Anchor::new(1.0, 1.0, 1.0, 1.0).unwrap());
	let largest = AppleActivity::ALL
		.into_iter()
		.fold(largest, Content::excluding);
	let bytes = largest.encode().unwrap();
	assert!(bytes.len() <= MAX_REQUEST_BYTES);
	assert_eq!(Content::decode(&bytes).unwrap(), largest);
	assert_eq!(MAX_REQUEST_BYTES, 4_418_112);
}

#[test]
fn decoded_files_apply_every_rule() {
	let decode = |source: u32, payload: &[u8], mime: &[u8], name: &[u8]| {
		let mut encoder = Encoder::default();
		encoder.u32(WIRE_VERSION);

		for _ in 0..4 {
			encoder.bytes(b"").unwrap();
		}

		encoder.u32(0);
		encoder.u32(0);
		encoder.u32(1);
		encoder.u32(source);
		encoder.bytes(payload).unwrap();
		encoder.bytes(mime).unwrap();
		encoder.bytes(name).unwrap();
		Content::decode(&encoder.finish())
	};
	assert!(decode(0, b"a.txt", b"", b"").is_ok());
	assert!(decode(0, b"a.txt", b"text/plain", b"shown.txt").is_ok());
	assert!(decode(1, b"\xFF\x00", b"", b"raw.bin").is_ok());
	assert_eq!(decode(1, b"x", b"", b""), Err(Error::InvalidArgument));
	assert_eq!(decode(2, b"a.txt", b"", b""), Err(Error::InvalidArgument));
	assert_eq!(decode(0, b"../a", b"", b""), Err(Error::InvalidArgument));
	assert_eq!(decode(0, b"\xFF", b"", b""), Err(Error::InvalidArgument));
	assert_eq!(decode(0, b"a", b"bad", b""), Err(Error::InvalidArgument));
	assert_eq!(decode(0, b"a", b"", b"a/b"), Err(Error::InvalidArgument));
	let mut count = GOLDEN_REQUEST.to_vec();
	count[52] = 33;
	assert_eq!(Content::decode(&count), Err(Error::TooLarge));
}
