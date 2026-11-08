//! Round-trips, snapshots, and conformance vectors for LK Pod.

#![allow(clippy::bool_assert_comparison)]
#![allow(clippy::format_collect)]
#![allow(clippy::format_push_string)]
#![allow(clippy::redundant_closure)]

use leptoskit_pod::Error;
use leptoskit_pod::MessageType;
use leptoskit_pod::PodDecode;
use leptoskit_pod::PodEncode;
use leptoskit_pod::Reader;
use leptoskit_pod::Writer;
use leptoskit_pod::decode_struct_version;
use leptoskit_pod::encode_struct_v0;
use leptoskit_pod::unwrap;
use leptoskit_pod::wrap;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Note {
	priority: u8,
	archived: bool,
	title: String,
}

impl PodEncode for Note {
	fn encode(&self, w: &mut Writer) -> leptoskit_pod::Result<()> {
		encode_struct_v0(w, |w| {
			self.priority.encode(w)?;
			self.archived.encode(w)?;
			self.title.encode(w)?;
			Ok(())
		})
	}
}

impl<'a> PodDecode<'a> for Note {
	fn decode(r: &mut Reader<'a>) -> leptoskit_pod::Result<Self> {
		decode_struct_version(r, 0, |r| {
			Ok(Self {
				priority: u8::decode(r)?,
				archived: bool::decode(r)?,
				title: String::decode(r)?,
			})
		})
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LiveUpdate {
	Ok,
	Patch { priority: u8, title: String },
}

impl PodEncode for LiveUpdate {
	fn encode(&self, w: &mut Writer) -> leptoskit_pod::Result<()> {
		match self {
			Self::Ok => {
				w.variant(0);
				Ok(())
			}
			Self::Patch { priority, title } => {
				w.variant(1);
				encode_struct_v0(w, |w| {
					priority.encode(w)?;
					title.encode(w)?;
					Ok(())
				})
			}
		}
	}
}

impl<'a> PodDecode<'a> for LiveUpdate {
	fn decode(r: &mut Reader<'a>) -> leptoskit_pod::Result<Self> {
		match r.variant()? {
			0 => Ok(Self::Ok),
			1 => {
				decode_struct_version(r, 0, |r| {
					Ok(Self::Patch {
						priority: u8::decode(r)?,
						title: String::decode(r)?,
					})
				})
			}
			other => Err(Error::InvalidTag(other)),
		}
	}
}

fn hex_encode(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(hex: &str) -> Vec<u8> {
	(0..hex.len())
		.step_by(2)
		.map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
		.collect()
}

#[test]
fn note_matches_design_bytes() {
	let note = Note {
		priority: 7,
		archived: true,
		title: "hi".into(),
	};
	let bytes = note.to_pod_bytes().unwrap();
	assert_eq!(hex_encode(&bytes), "00000701020000006869");
	assert_eq!(Note::from_pod_bytes(&bytes).unwrap(), note);
}

#[test]
fn note_snapshot() {
	let note = Note {
		priority: 7,
		archived: true,
		title: "hi".into(),
	};
	insta::assert_snapshot!(hex_encode(&note.to_pod_bytes().unwrap()), @"00000701020000006869");
}

#[test]
fn primitives_round_trip() {
	assert_eq!(u8::from_pod_bytes(&7u8.to_pod_bytes().unwrap()).unwrap(), 7);
	assert_eq!(
		u64::from_pod_bytes(&0x1122_3344_5566_7788u64.to_pod_bytes().unwrap()).unwrap(),
		0x1122_3344_5566_7788
	);
	assert_eq!(
		bool::from_pod_bytes(&true.to_pod_bytes().unwrap()).unwrap(),
		true
	);
	assert_eq!(
		String::from_pod_bytes(&"abc".to_owned().to_pod_bytes().unwrap()).unwrap(),
		"abc"
	);
	assert_eq!(
		Option::<u8>::from_pod_bytes(&None::<u8>.to_pod_bytes().unwrap()).unwrap(),
		None
	);
	assert_eq!(
		Option::<u8>::from_pod_bytes(&Some(9u8).to_pod_bytes().unwrap()).unwrap(),
		Some(9)
	);
	assert_eq!(
		Result::<u8, u8>::from_pod_bytes(&Ok::<u8, u8>(3).to_pod_bytes().unwrap()).unwrap(),
		Ok(3)
	);
	assert_eq!(
		Result::<u8, u8>::from_pod_bytes(&Err::<u8, u8>(4).to_pod_bytes().unwrap()).unwrap(),
		Err(4)
	);
	assert_eq!(
		Vec::<u8>::from_pod_bytes(&vec![1u8, 2, 3].to_pod_bytes().unwrap()).unwrap(),
		vec![1, 2, 3]
	);
}

#[test]
fn enum_round_trip() {
	let ok = LiveUpdate::Ok;
	assert_eq!(
		LiveUpdate::from_pod_bytes(&ok.to_pod_bytes().unwrap()).unwrap(),
		ok
	);
	let patch = LiveUpdate::Patch {
		priority: 7,
		title: "abc".into(),
	};
	assert_eq!(
		hex_encode(&patch.to_pod_bytes().unwrap()),
		"010000000703000000616263"
	);
	assert_eq!(
		LiveUpdate::from_pod_bytes(&patch.to_pod_bytes().unwrap()).unwrap(),
		patch
	);
}

#[test]
fn envelope_round_trip() {
	let body = hex_decode("00000701020000006869");
	let framed = wrap(MessageType::Rpc, 0, &body).unwrap();
	assert_eq!(&framed[..4], b"LK01");
	let env = unwrap(&framed).unwrap();
	assert_eq!(env.msg_type, MessageType::Rpc);
	assert_eq!(env.body, body.as_slice());
}

#[test]
fn rejects_trailing_and_truncation() {
	assert_eq!(u8::from_pod_bytes(&[1, 2]).unwrap_err(), Error::Trailing);
	assert_eq!(Reader::new(&[1]).u16().unwrap_err(), Error::Truncated);
	assert_eq!(
		Reader::new(&[1, 0, 0, 0, 0xff]).string().unwrap_err(),
		Error::InvalidUtf8
	);
	assert_eq!(Reader::new(&[2]).bool().unwrap_err(), Error::InvalidTag(2));
}

#[test]
fn rejects_bad_envelope() {
	assert_eq!(unwrap(&[0; 14]).unwrap_err(), Error::InvalidEnvelope);
	assert_eq!(unwrap(b"LK01").unwrap_err(), Error::Truncated);
}

#[derive(Debug, Deserialize)]
struct Corpus {
	vectors: Vec<Vector>,
	malformed: Vec<Malformed>,
}

#[derive(Debug, Deserialize)]
struct Vector {
	name: String,
	hex: String,
	#[serde(default)]
	envelope: bool,
}

#[derive(Debug, Deserialize)]
struct Malformed {
	name: String,
	hex: String,
	expect: String,
	#[serde(default)]
	envelope: bool,
}

#[test]
fn conformance_vectors_stable() {
	let raw = include_str!("../conformance/vectors.json");
	let corpus: Corpus = serde_json::from_str(raw).unwrap();
	let mut report = String::new();
	for v in &corpus.vectors {
		let bytes = hex_decode(&v.hex);
		report.push_str(&format!("{}: {}\n", v.name, v.hex));
		if v.envelope {
			unwrap(&bytes).unwrap_or_else(|e| panic!("{}: {e}", v.name));
		}
	}
	for m in &corpus.malformed {
		let bytes = hex_decode(&m.hex);
		let err = if m.envelope {
			unwrap(&bytes).unwrap_err()
		} else if m.name.contains("utf8") {
			Reader::new(&bytes).string().unwrap_err()
		} else if m.name.contains("bool") {
			Reader::new(&bytes).bool().unwrap_err()
		} else if m.name.contains("option") {
			Reader::new(&bytes).option_tag().unwrap_err()
		} else if m.name.contains("result") {
			Reader::new(&bytes).result_tag().unwrap_err()
		} else if m.name.contains("vec") {
			Vec::<u8>::from_pod_bytes(&bytes).unwrap_err()
		} else {
			Reader::new(&bytes).u16().unwrap_err()
		};
		let label = format!("{err:?}");
		assert!(
			label.contains(&m.expect),
			"{}: expected {}, got {label}",
			m.name,
			m.expect
		);
		report.push_str(&format!("{}: {}\n", m.name, m.expect));
	}
	insta::assert_snapshot!(report);
}

#[test]
fn writer_helpers_cover_signed() {
	let mut w = Writer::new();
	w.i8(-1);
	w.i16(-2);
	w.i32(-3);
	w.i64(-4);
	w.vec_len(0).unwrap();
	assert!(!w.is_empty());
	assert_eq!(w.len(), 1 + 2 + 4 + 8 + 4);
	let bytes = w.finish();
	let mut r = Reader::new(&bytes);
	assert_eq!(r.i8().unwrap(), -1);
	assert_eq!(r.i16().unwrap(), -2);
	assert_eq!(r.i32().unwrap(), -3);
	assert_eq!(r.i64().unwrap(), -4);
	assert_eq!(r.vec_len().unwrap(), 0);
	r.finish().unwrap();
}

#[test]
fn unknown_struct_version_fails() {
	let mut w = Writer::new();
	w.struct_header(9, 0);
	w.u8(1);
	let bytes = w.finish();
	let err = decode_struct_version(&mut Reader::new(&bytes), 0, |r| u8::decode(r)).unwrap_err();
	assert_eq!(err, Error::UnknownVersion(9));
}
