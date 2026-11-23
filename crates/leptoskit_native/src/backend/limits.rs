//! Count serialized batches before retaining another owned mutation copy.

use std::io;
use std::io::Write;
use std::sync::LazyLock;

use super::Mutation;
use crate::props::NativeKind;
use crate::props::NativeProps;

static EMPTY_PROPS: LazyLock<NativeProps> = LazyLock::new(NativeProps::default);

pub(super) struct LogBudget {
	limit: usize,
	remaining: usize,
	exceeded: bool,
}

impl LogBudget {
	pub fn new(limit: usize) -> Self {
		Self {
			limit,
			remaining: limit,
			exceeded: false,
		}
	}

	pub fn admit(&mut self, mutations: &[Mutation]) -> bool {
		if mutations.is_empty() {
			return !self.exceeded;
		}

		if self.exceeded {
			return false;
		}

		// JSON array framing: `[` + `]` + (len - 1) commas = len + 1 bytes.
		let framing = mutations.len() + 1;

		if framing > self.remaining {
			self.exceeded = true;
			return false;
		}

		self.remaining -= framing;

		for mutation in mutations {
			if !self.charge_mutation(mutation) {
				self.exceeded = true;
				return false;
			}
		}

		true
	}

	#[inline]
	fn charge_bytes(&mut self, bytes: usize) -> bool {
		if bytes > self.remaining {
			return false;
		}

		self.remaining -= bytes;
		true
	}

	#[inline]
	fn charge_mutation(&mut self, mutation: &Mutation) -> bool {
		match mutation {
			Mutation::Delete { id } => {
				// `{"Delete":{"id":` (16) + digits + `}}` (2)
				self.charge_bytes(18 + usize_digits(id.index()))
			}
			Mutation::RemoveChild { parent, child } => {
				// `{"RemoveChild":{"parent":` (25) + p + `,"child":` (9) + c +
				// `}}` (2)
				self.charge_bytes(36 + usize_digits(parent.index()) + usize_digits(child.index()))
			}
			Mutation::InsertChild {
				parent,
				child,
				index,
			} => {
				// `{"InsertChild":{"parent":` (25) + p + `,"child":` (9) + c +
				// `,"index":` (9) + i + `}}` (2)
				self.charge_bytes(
					45 + usize_digits(parent.index())
						+ usize_digits(child.index())
						+ usize_digits(*index),
				)
			}
			Mutation::UpdateLayout {
				id,
				x,
				y,
				width,
				height,
			} => {
				if let (Some(lx), Some(ly), Some(lw), Some(lh)) = (
					fast_f32_json_len(*x),
					fast_f32_json_len(*y),
					fast_f32_json_len(*width),
					fast_f32_json_len(*height),
				) {
					// `{"UpdateLayout":{"id":` (22) + id + `,"x":` (5) + x +
					// `,"y":` (5) + y
					// + `,"width":` (9) + w + `,"height":` (10) + h + `}}` (2)
					//   = 53
					self.charge_bytes(53 + usize_digits(id.index()) + lx + ly + lw + lh)
				} else {
					serde_json::to_writer(&mut *self, mutation).is_ok()
				}
			}
			Mutation::Create { id, kind, props } => {
				if matches!(kind, NativeKind::Custom(_)) {
					return serde_json::to_writer(&mut *self, mutation).is_ok();
				}

				// `{"Create":{"id":` (16) + id + `,"kind":"` (9) + wire_name +
				// `","props":` (10) + props + `}}` (2) = 37
				let prefix = 37 + usize_digits(id.index()) + kind.wire_name().len();

				if props.motion.is_none() && *props == *EMPTY_PROPS {
					self.charge_bytes(prefix + 2)
				} else {
					self.charge_bytes(prefix) && serde_json::to_writer(&mut *self, props).is_ok()
				}
			}
			Mutation::UpdateProps { id, props } => {
				// `{"UpdateProps":{"id":` (21) + id + `,"props":` (9) + props +
				// `}}` (2) = 32
				let prefix = 32 + usize_digits(id.index());

				if props.motion.is_none() && *props == *EMPTY_PROPS {
					self.charge_bytes(prefix + 2)
				} else {
					self.charge_bytes(prefix) && serde_json::to_writer(&mut *self, props).is_ok()
				}
			}
		}
	}

	pub fn exceeded(&self) -> bool {
		self.exceeded
	}

	pub fn restrict(&mut self, limit: usize) {
		let spent = self.limit - self.remaining;
		self.limit = self.limit.min(limit);
		self.remaining = self.limit.saturating_sub(spent);
		self.exceeded |= spent > self.limit;
	}

	pub fn drain(&mut self) {
		self.remaining = self.limit;
	}
}

#[inline]
const fn usize_digits(n: usize) -> usize {
	if n < 10 {
		1
	} else if n < 100 {
		2
	} else if n < 1_000 {
		3
	} else if n < 10_000 {
		4
	} else if n < 100_000 {
		5
	} else if n < 1_000_000 {
		6
	} else {
		let mut value = n / 1_000_000;
		let mut digits = 6;

		while value > 0 {
			digits += 1;
			value /= 10;
		}

		digits
	}
}

/// Exact `serde_json` (ryu) byte length for common layout coordinates:
/// non-negative finite integers strictly below `1_000_000.0`, which `ryu`
/// formats as `<digits>.0`. Returns `None` for negative zero, negative
/// values, fractions, large exponents, or non-finite floats so the caller
/// falls back to `serde_json::to_writer`.
#[inline]
fn fast_f32_json_len(value: f32) -> Option<usize> {
	if value.to_bits() == 0 {
		return Some(3); // "0.0"
	}

	if value > 0.0 && value < 1_000_000.0 && value.fract() == 0.0 {
		Some(usize_digits(value as usize) + 2)
	} else {
		None
	}
}

impl Write for LogBudget {
	#[inline]
	fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
		if bytes.len() > self.remaining {
			return Err(io::Error::other("render mutation limit exceeded"));
		}

		self.remaining -= bytes.len();
		Ok(bytes.len())
	}

	#[inline]
	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::HeadlessBackend;
	use crate::MutationList;
	use crate::NativeBackend;
	use crate::props::Color;

	#[test]
	fn draining_returns_capacity_but_cannot_revive_a_failed_recorder() {
		let id = leptoskit_yoga::Yoga::new().create_node();
		let mut batch = MutationList::new();
		batch.push(Mutation::UpdateLayout {
			id,
			x: 0.0,
			y: 0.0,
			width: 1.0,
			height: 1.0,
		});
		let size = serde_json::to_vec(batch.mutations()).unwrap().len();
		let mut backend = HeadlessBackend::new().with_log_limit(size);
		backend.apply(&batch);
		assert_eq!(backend.take_log().len(), 1);
		backend.apply(&batch);

		assert!(backend.render_error().is_none());
		backend.limit_mutation_log(size * 2);
		backend.apply(&batch);
		assert!(backend.render_error().is_some());
		assert_eq!(
			backend.take_log().len(),
			1,
			"an excessive batch is rejected atomically"
		);
		backend.limit_mutation_log(usize::MAX);
		backend.apply(&batch);
		assert_eq!(backend.log(), []);

		assert!(backend.render_error().is_some());
	}

	#[test]
	fn fast_byte_accounting_matches_serde_json_exactly() {
		let mut yoga = leptoskit_yoga::Yoga::new();
		let n0 = yoga.create_node();
		let n1 = yoga.create_node();
		let mut styled = NativeProps::text("hello \"world\"");
		styled.color = Some(Color(0x1122_33FF));
		let mut motion_only = NativeProps::default();
		motion_only.motion = Some(Box::default());

		let cases = vec![
			Mutation::Delete { id: n0 },
			Mutation::RemoveChild {
				parent: n0,
				child: n1,
			},
			Mutation::InsertChild {
				parent: n0,
				child: n1,
				index: 42,
			},
			Mutation::UpdateLayout {
				id: n1,
				x: 0.0,
				y: 12.0,
				width: 390.0,
				height: 999_999.0,
			},
			Mutation::UpdateLayout {
				id: n1,
				x: -0.0,
				y: 1.5,
				width: 1_000_000.0,
				height: -42.25,
			},
			Mutation::Create {
				id: n0,
				kind: NativeKind::View,
				props: NativeProps::default(),
			},
			Mutation::Create {
				id: n1,
				kind: NativeKind::Text,
				props: styled.clone(),
			},
			Mutation::Create {
				id: n1,
				kind: NativeKind::Custom("acme\"widget"),
				props: NativeProps::default(),
			},
			Mutation::UpdateProps {
				id: n0,
				props: NativeProps::default(),
			},
			Mutation::UpdateProps {
				id: n0,
				props: motion_only,
			},
			Mutation::UpdateProps {
				id: n1,
				props: styled,
			},
		];

		for mutation in &cases {
			let slice = std::slice::from_ref(mutation);
			let expected = serde_json::to_vec(slice).unwrap().len();
			let mut budget = LogBudget::new(expected);
			assert!(
				budget.admit(slice),
				"exact budget should admit {mutation:?} ({expected} bytes)"
			);
			assert_eq!(
				budget.remaining, 0,
				"exact budget should leave 0 bytes for {mutation:?}"
			);
			let mut short = LogBudget::new(expected - 1);
			assert!(
				!short.admit(slice),
				"one byte short must reject {mutation:?}"
			);
		}

		// Also verify multi-element array framing matches serde_json::to_vec.
		let expected_all = serde_json::to_vec(&cases).unwrap().len();
		let mut budget = LogBudget::new(expected_all);
		assert!(budget.admit(&cases));
		assert_eq!(budget.remaining, 0);
	}
}
