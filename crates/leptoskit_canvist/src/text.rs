//! Text arithmetic the browser host needs and no browser has to run:
//! grapheme clusters, CSS `font` shorthands, ligature edges, and words.
//!
//! It is kept apart from the host so it is tested on every target.

use unicode_segmentation::UnicodeSegmentation;

/// Characters below this are never part of a multi-character grapheme
/// cluster: combining marks start at U+0300, and joiners, variation
/// selectors, and everything outside the basic plane are above it.
const SIMPLE_TEXT_LIMIT: u32 = 0x300;

/// A sequence a face draws as one glyph. Browsers place caret stops inside
/// a ligature at fractions of its width (in equal parts, unless the font
/// says otherwise), not where each letter's own advance would end.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ligature {
	/// The letters, all ASCII.
	pub(crate) text: &'static str,
	/// Each inner caret edge, as a share of the ligature's width.
	pub(crate) edges: Vec<f64>,
}

/// Scale `advances` so they sum to `total`.
pub(crate) fn scale(advances: &mut [f64], total: f64) {
	let sum: f64 = advances.iter().sum();

	if sum <= 0.0 {
		return;
	}

	let factor = total / sum;

	for advance in advances {
		*advance *= factor;
	}
}

/// Move the edges inside each ligature in `clusters` to where the DOM puts
/// them: at the ligature's fractions of the width between its outer edges,
/// which stay where the canvas measured them.
pub(crate) fn split_ligatures(clusters: &[&str], edges: &mut [f64], ligatures: &[Ligature]) {
	let mut index = 0;

	while index < clusters.len() && !ligatures.is_empty() {
		let found = ligatures.iter().find(|ligature| {
			// One byte per letter, so a byte offset counts clusters.
			ligature.text.char_indices().all(|(offset, letter)| {
				clusters
					.get(index + offset)
					.is_some_and(|cluster| cluster.chars().eq([letter]))
			})
		});

		let Some(ligature) = found else {
			index += 1;
			continue;
		};

		let start = if index == 0 { 0.0 } else { edges[index - 1] };
		let last = index + ligature.text.len() - 1;
		let width = edges[last] - start;

		for (offset, share) in ligature.edges.iter().enumerate() {
			edges[index + offset] = start + share * width;
		}

		index = last + 1;
	}
}

/// The grapheme clusters of `segment`.
pub(crate) fn clusters(segment: &str) -> Vec<&str> {
	if segment
		.chars()
		.all(|character| u32::from(character) < SIMPLE_TEXT_LIMIT)
	{
		return segment
			.char_indices()
			.map(|(start, character)| &segment[start..start + character.len_utf8()])
			.collect();
	}

	segment.graphemes(true).collect()
}

/// Whether `character` is in a script whose letters join: Arabic, Syriac,
/// Thaana, N'Ko, Mandaic, and Mongolian.
pub(crate) fn joins(character: char) -> bool {
	matches!(
		u32::from(character),
		0x0600..=0x08ff | 0x1800..=0x18af | 0xfb50..=0xfdff | 0xfe70..=0xfeff
	)
}

/// A CSS `font` shorthand as layout writes it (`normal 400 16px Inter,
/// serif`), split into what comes before the family list and the list.
pub(crate) fn split_shorthand(css: &str) -> Option<(&str, &str)> {
	let mut spaces = css.match_indices(' ').map(|(index, _)| index);
	let (_, weight_end, size_end) = (spaces.next()?, spaces.next()?, spaces.next()?);
	let (prefix, list) = (&css[..size_end], &css[size_end + 1..]);

	(prefix[weight_end + 1..].ends_with("px") && !list.is_empty()).then_some((prefix, list))
}

/// `font` (a CSS `font` shorthand) at `size`, a CSS length such as `200px`.
pub(crate) fn at_size(font: &str, size: &str) -> String {
	let Some((prefix, list)) = split_shorthand(font) else {
		return font.to_owned();
	};
	let style_and_weight = prefix.rsplit_once(' ').map_or(prefix, |(head, _)| head);

	format!("{style_and_weight} {size} {list}")
}

/// The families of a CSS `font-family` list, splitting on commas outside
/// quotes.
pub(crate) fn split_families(list: &str) -> Vec<&str> {
	let mut families = Vec::new();
	let mut start = 0;
	let mut quote = None;

	for (index, character) in list.char_indices() {
		match (quote, character) {
			(Some(open), close) if close == open => quote = None,
			(None, '"' | '\'') => quote = Some(character),
			(None, ',') => {
				families.push(list[start..index].trim());
				start = index + 1;
			}
			_ => {}
		}
	}

	families.push(list[start..].trim());
	families.retain(|family| !family.is_empty());
	families
}

/// The word, run of spaces, or punctuation that byte `offset` is in or just
/// before, as a byte range.
pub(crate) fn word_around(text: &str, offset: u32) -> (u32, u32) {
	let offset = offset as usize;
	let mut last = (0, 0);

	for (start, segment) in text.split_word_bound_indices() {
		let end = start + segment.len();
		last = (start as u32, end as u32);

		if offset < end {
			return last;
		}
	}

	last
}

#[cfg(test)]
#[allow(
	clippy::float_cmp,
	reason = "the values compared are exactly representable"
)]
mod tests {
	use super::*;

	#[test]
	fn shorthands_split_before_the_family_list() {
		assert_eq!(
			split_shorthand("italic 700 16.5px Inter, \"Noto Sans\", serif"),
			Some(("italic 700 16.5px", "Inter, \"Noto Sans\", serif"))
		);
		assert_eq!(split_shorthand("16px serif"), None);
		assert_eq!(split_shorthand("normal 400 1em serif"), None);
		assert_eq!(
			at_size("normal 400 16px Inter, serif", "200px"),
			"normal 400 200px Inter, serif"
		);
		assert_eq!(at_size("caption", "200px"), "caption");
	}

	#[test]
	fn family_lists_split_outside_quotes() {
		assert_eq!(
			split_families("Inter, \"Font, With Comma\" , 'Other', serif,"),
			["Inter", "\"Font, With Comma\"", "'Other'", "serif"]
		);
	}

	#[test]
	fn clusters_keep_graphemes_whole() {
		assert_eq!(clusters("héy"), ["h", "é", "y"]);
		assert_eq!(clusters("e\u{301}👍🏽"), ["e\u{301}", "👍🏽"]);
		assert_eq!(clusters(""), [] as [&str; 0]);
	}

	#[test]
	fn joining_scripts_are_recognised() {
		assert!("سلام".chars().all(joins));
		assert!(!"hello שלום".chars().any(joins));
	}

	#[test]
	fn ligature_edges_move_to_the_shares_the_dom_uses() {
		let ligatures = [
			Ligature {
				text: "ffi",
				edges: vec![1.0 / 3.0, 2.0 / 3.0],
			},
			Ligature {
				text: "fi",
				edges: vec![0.5],
			},
		];
		// "afib": the canvas put the edge inside "fi" at 14, the DOM halves
		// the ligature's 10 to 20.
		let mut edges = vec![10.0, 14.0, 20.0, 30.0];
		split_ligatures(&["a", "f", "i", "b"], &mut edges, &ligatures);

		assert_eq!(edges, [10.0, 15.0, 20.0, 30.0]);

		// A ligature at the start measures from zero, and one cut short at
		// the end of the text is no ligature.
		let mut edges = vec![4.0, 8.0, 12.0];
		split_ligatures(&["f", "i", "f"], &mut edges, &ligatures);

		assert_eq!(edges, [4.0, 8.0, 12.0]);
	}

	#[test]
	fn scaling_keeps_proportions() {
		let mut advances = [1.0, 3.0];
		scale(&mut advances, 8.0);

		assert_eq!(advances, [2.0, 6.0]);

		let mut nothing = [0.0, 0.0];
		scale(&mut nothing, 8.0);

		assert_eq!(nothing, [0.0, 0.0]);
	}

	#[test]
	fn words_around_offsets() {
		let text = "hello, wide world";

		assert_eq!(word_around(text, 2), (0, 5));
		assert_eq!(word_around(text, 5), (5, 6), "the comma");
		assert_eq!(word_around(text, 9), (7, 11));
		assert_eq!(
			word_around(text, text.len() as u32),
			(12, 17),
			"the end selects the last word"
		);
		assert_eq!(word_around("", 0), (0, 0));
	}
}
