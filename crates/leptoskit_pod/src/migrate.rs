//! Adjacent struct-version transitions (Pina-style).
//!
//! Historical Pod bytes decode at their on-wire version, then climb through
//! checked-in transitions to the current layout before the handler runs.
//!
//! **RPC / jobs:** migrate-up inbound args; responses encode at the current
//! version (old clients may fail to decode replies — that is intentional).
//! **Live / events:** version-not-migrate; keep historical decoders.

use crate::Error;
use crate::Reader;
use crate::Result;
use crate::Writer;

/// One adjacent rewrite: `from` → `from + 1`.
pub trait Transition {
	/// Source version this step consumes.
	#[allow(clippy::wrong_self_convention)]
	fn from_version(&self) -> u16;

	/// Rewrites payload bytes for `from_version` into `from_version + 1`.
	///
	/// `old` is the struct **payload only** — no version header.
	fn migrate(&self, old: &[u8]) -> Result<Vec<u8>>;
}

/// Climbs a full struct (`[version:u16][payload…]`) from its on-wire version
/// up to `current` using adjacent `steps`.
///
/// Rejects a version ahead of `current` or a gap in the transition table.
/// Identity when the on-wire version already equals `current`.
pub fn climb(bytes: &[u8], current: u16, steps: &[&dyn Transition]) -> Result<Vec<u8>> {
	let mut r = Reader::new(bytes);
	let mut version = r.struct_header()?;
	let mut payload = r.remaining().to_vec();
	while version < current {
		let step = steps
			.iter()
			.find(|s| s.from_version() == version)
			.ok_or(Error::UnknownVersion(version))?;
		payload = step.migrate(&payload)?;
		version = version.checked_add(1).ok_or(Error::TooLarge)?;
	}
	if version != current {
		return Err(Error::UnknownVersion(version));
	}
	let mut out = Writer::with_capacity(2 + payload.len());
	out.struct_header(current);
	out.raw(&payload);
	Ok(out.finish())
}

#[cfg(test)]
mod tests {
	use super::*;

	struct V0ToV1;

	impl Transition for V0ToV1 {
		fn from_version(&self) -> u16 {
			0
		}

		fn migrate(&self, old: &[u8]) -> Result<Vec<u8>> {
			// v0: priority:u8 ; v1: priority:u8 + archived:bool=false
			let mut r = Reader::new(old);
			let priority = r.u8()?;
			r.finish()?;
			let mut w = Writer::new();
			w.u8(priority);
			w.bool(false);
			Ok(w.finish())
		}
	}

	#[test]
	fn climbs_one_step() {
		let mut w = Writer::new();
		w.struct_header(0);
		w.u8(7);
		let old = w.finish();
		let next = climb(&old, 1, &[&V0ToV1]).unwrap();
		let mut r = Reader::new(&next);
		assert_eq!(r.struct_header().unwrap(), 1);
		assert_eq!(r.u8().unwrap(), 7);
		assert!(!r.bool().unwrap());
		r.finish().unwrap();
	}

	#[test]
	fn identity_when_already_current() {
		let mut w = Writer::new();
		w.struct_header(1);
		w.u8(3);
		w.bool(true);
		let current = w.finish();
		assert_eq!(climb(&current, 1, &[&V0ToV1]).unwrap(), current);
	}
}
