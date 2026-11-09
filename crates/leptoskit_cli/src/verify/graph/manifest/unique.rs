//! Reject duplicate keys at every level before interpreting signed/pinned
//! JSON. In particular, an asset path cannot silently replace another entry.

use std::collections::BTreeSet;
use std::fmt;

use serde::Deserialize;
use serde::Deserializer;
use serde::de::Error as _;
use serde::de::MapAccess;
use serde::de::SeqAccess;
use serde::de::Visitor;

pub(super) struct Unique;

impl<'de> Deserialize<'de> for Unique {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		deserializer.deserialize_any(Check)
	}
}

struct Check;

impl<'de> Visitor<'de> for Check {
	type Value = Unique;

	fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str("JSON with unique object keys")
	}

	fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Unique, A::Error> {
		let mut keys = BTreeSet::new();
		while let Some(key) = map.next_key::<String>()? {
			if !keys.insert(key) {
				return Err(A::Error::custom("duplicate JSON key"));
			}
			map.next_value::<Unique>()?;
		}
		Ok(Unique)
	}

	fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Unique, A::Error> {
		while seq.next_element::<Unique>()?.is_some() {}
		Ok(Unique)
	}

	fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Unique, E> {
		Ok(Unique)
	}

	fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Unique, E> {
		Ok(Unique)
	}

	fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Unique, E> {
		Ok(Unique)
	}

	fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Unique, E> {
		Ok(Unique)
	}

	fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Unique, E> {
		Ok(Unique)
	}

	fn visit_unit<E: serde::de::Error>(self) -> Result<Unique, E> {
		Ok(Unique)
	}
}
