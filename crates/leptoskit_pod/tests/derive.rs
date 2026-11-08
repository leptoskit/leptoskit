//! `#[pod::data]` derive coverage.

#![allow(clippy::format_collect)]

use leptoskit_pod::PodDecode;
use leptoskit_pod::PodEncode;
use leptoskit_pod::pod;

#[pod::data]
#[derive(Debug, Clone, PartialEq, Eq)]
struct Note {
	priority: u8,
	archived: bool,
	title: String,
}

#[pod::data]
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
	Ready,
	Failed { code: u16, message: String },
	Retry(u8),
}

#[test]
fn derived_note_matches_hand_written() {
	let note = Note {
		priority: 7,
		archived: true,
		title: "hi".into(),
	};
	let bytes = note.to_pod_bytes().unwrap();
	assert_eq!(
		bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
		"00000701020000006869"
	);
	assert_eq!(Note::from_pod_bytes(&bytes).unwrap(), note);
}

#[test]
fn derived_enum_variants() {
	let ready = Status::Ready;
	assert_eq!(
		Status::from_pod_bytes(&ready.to_pod_bytes().unwrap()).unwrap(),
		ready
	);

	let failed = Status::Failed {
		code: 404,
		message: "gone".into(),
	};
	assert_eq!(
		Status::from_pod_bytes(&failed.to_pod_bytes().unwrap()).unwrap(),
		failed
	);

	let retry = Status::Retry(3);
	assert_eq!(
		Status::from_pod_bytes(&retry.to_pod_bytes().unwrap()).unwrap(),
		retry
	);
}
