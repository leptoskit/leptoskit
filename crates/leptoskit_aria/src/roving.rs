//! One Tab stop for a group, with the arrow keys moving inside it.

use leptoskit_native::KeyPress;
use leptoskit_native::Orientation;

/// The item a key press moves to from `current` in a group of `count`, or
/// `None` when the key is not one the group takes.
///
/// The arrows along `orientation` step to the next or previous item and wrap
/// at the ends; `None` takes all four arrows, as a radio group does. Home and
/// End go to the first and last item. A key held with a modifier is left to
/// whatever else wants it.
pub(crate) fn target(
	key: &KeyPress,
	orientation: Option<Orientation>,
	current: usize,
	count: usize,
) -> Option<usize> {
	if count == 0 || key.ctrl || key.alt || key.shift {
		return None;
	}

	let horizontal = orientation != Some(Orientation::Vertical);
	let vertical = orientation != Some(Orientation::Horizontal);

	match key.key.as_str() {
		"ArrowRight" if horizontal => Some((current + 1) % count),
		"ArrowDown" if vertical => Some((current + 1) % count),
		"ArrowLeft" if horizontal => Some((current + count - 1) % count),
		"ArrowUp" if vertical => Some((current + count - 1) % count),
		"Home" => Some(0),
		"End" => Some(count - 1),
		_ => None,
	}
}

/// The item a key press lands on in a group of `count` where nothing is
/// current yet: forward keys and Home enter at the first item, backward keys
/// and End at the last.
pub(crate) fn entry(key: &KeyPress, count: usize) -> Option<usize> {
	// Stepping forward from the last item, or backward from the first,
	// wraps to the end the key points at.
	match key.key.as_str() {
		"ArrowRight" | "ArrowDown" => target(key, None, count.checked_sub(1)?, count),
		_ => target(key, None, 0, count),
	}
}

/// The `tabindex` of an item in a roving group: the current item is the
/// group's one Tab stop, and the others are reached with the arrow keys.
pub(crate) const fn tab_index(current: bool) -> i32 {
	if current { 0 } else { -1 }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn arrows_follow_the_orientation_and_wrap() {
		let press = |key: &str, orientation| target(&KeyPress::new(key), orientation, 2, 3);
		let across = Some(Orientation::Horizontal);
		let down = Some(Orientation::Vertical);

		assert_eq!(press("ArrowRight", across), Some(0));
		assert_eq!(press("ArrowLeft", across), Some(1));
		assert_eq!(press("ArrowDown", across), None);
		assert_eq!(press("ArrowDown", down), Some(0));
		assert_eq!(press("ArrowUp", down), Some(1));
		assert_eq!(press("ArrowRight", down), None);
		assert_eq!(press("ArrowDown", None), Some(0));
		assert_eq!(press("ArrowLeft", None), Some(1));
		assert_eq!(press("Home", across), Some(0));
		assert_eq!(press("End", down), Some(2));
		assert_eq!(press("Enter", None), None);
	}

	#[test]
	fn a_group_with_nothing_current_is_entered_at_an_end() {
		let press = |key: &str| entry(&KeyPress::new(key), 3);

		assert_eq!(press("ArrowRight"), Some(0));
		assert_eq!(press("ArrowDown"), Some(0));
		assert_eq!(press("Home"), Some(0));
		assert_eq!(press("ArrowLeft"), Some(2));
		assert_eq!(press("ArrowUp"), Some(2));
		assert_eq!(press("End"), Some(2));
		assert_eq!(press("a"), None);
		assert_eq!(entry(&KeyPress::new("ArrowLeft"), 1), Some(0));
		assert_eq!(entry(&KeyPress::new("ArrowUp"), 2), Some(1));
		assert_eq!(entry(&KeyPress::new("ArrowDown"), 2), Some(0));
		assert_eq!(entry(&KeyPress::new("ArrowDown"), 0), None);
	}

	#[test]
	fn modified_keys_and_empty_groups_are_left_alone() {
		assert_eq!(
			target(&KeyPress::new("ArrowRight").ctrl(), None, 0, 3),
			None
		);
		assert_eq!(target(&KeyPress::new("Home").shift(), None, 1, 3), None);
		assert_eq!(target(&KeyPress::new("End").alt(), None, 1, 3), None);
		assert_eq!(target(&KeyPress::new("ArrowRight"), None, 0, 0), None);
	}
}
