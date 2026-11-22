//! Generates `llms.txt`, `llms-full.txt`, and plain-Markdown chapter copies so
//! coding agents and LLMs can discover and ingest every page of a built book.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::Chapter;
use crate::Site;
use crate::Summary;
use crate::SummaryItem;

/// Filename of the structured LLM index written at the built site's root.
pub(crate) const INDEX_FILE: &str = "llms.txt";

/// Filename of the concatenated full-book Markdown written at the site's root.
pub(crate) const FULL_FILE: &str = "llms-full.txt";

/// One chapter's cleaned Markdown and one-line summary, keyed by source path.
pub(crate) struct ChapterText<'a> {
	pub(crate) source: &'a str,
	pub(crate) summary: Option<String>,
	pub(crate) markdown: String,
}

/// Removes MDT template directive comments (`<!-- {=...} -->` and
/// `<!-- {/...} -->`) so LLM consumers read clean Markdown.
pub(crate) fn clean_markdown(source: &str) -> String {
	let mut out = String::with_capacity(source.len());
	let mut blank_run = 0usize;

	for line in source.lines() {
		let trimmed = line.trim();
		if is_mdt_marker(trimmed) {
			continue;
		}
		if trimmed.is_empty() {
			blank_run += 1;
			if blank_run > 2 {
				continue;
			}
		} else {
			blank_run = 0;
		}
		out.push_str(line.trim_end());
		out.push('\n');
	}

	out.trim().to_owned() + "\n"
}

fn is_mdt_marker(trimmed: &str) -> bool {
	trimmed.starts_with("<!-- {") && trimmed.ends_with("} -->")
}

/// Renders the structured `llms.txt` index following the llmstxt.org layout.
pub(crate) fn render_index(site: &Site, summary: &Summary, chapters: &[ChapterText<'_>]) -> String {
	let prefix = site.url.as_deref().unwrap_or("");
	let summaries: HashMap<&str, &str> = chapters
		.iter()
		.filter_map(|entry| entry.summary.as_deref().map(|text| (entry.source, text)))
		.collect();

	let mut out = String::new();
	let _ = writeln!(out, "# {}\n", site.title);

	if let Some(description) = site.description.as_deref() {
		let _ = writeln!(out, "> {description}\n");
	}

	let _ = writeln!(
		out,
		"This index lists every chapter as plain Markdown so coding agents and LLMs can fetch \
		 individual topics or read the entire book in one file:\n"
	);
	let _ = writeln!(
		out,
		"- [Full documentation]({prefix}{FULL_FILE}): Complete single-file Markdown reference \
		 covering every chapter.\n"
	);

	let mut opened_section = false;

	for item in &summary.items {
		match item {
			SummaryItem::Part(part) => {
				if opened_section {
					out.push('\n');
				}
				let _ = writeln!(out, "## {part}\n");
				opened_section = true;
			}
			SummaryItem::Chapter(chapter) => {
				if !opened_section {
					out.push_str("## Overview\n\n");
					opened_section = true;
				}
				append_chapter_index(&mut out, chapter, 0, prefix, &summaries);
			}
			SummaryItem::Separator => {}
		}
	}

	out
}

fn append_chapter_index(
	out: &mut String,
	chapter: &Chapter,
	depth: usize,
	prefix: &str,
	summaries: &HashMap<&str, &str>,
) {
	if let Some(path) = chapter.path.as_deref() {
		let indent = "  ".repeat(depth);
		match summaries.get(path) {
			Some(desc) if !desc.is_empty() => {
				let _ = writeln!(out, "{indent}- [{}]({prefix}{path}): {desc}", chapter.title);
			}
			_ => {
				let _ = writeln!(out, "{indent}- [{}]({prefix}{path})", chapter.title);
			}
		}
	}

	for child in &chapter.children {
		append_chapter_index(out, child, depth + 1, prefix, summaries);
	}
}

/// Renders the single-file `llms-full.txt` concatenation in reading order.
pub(crate) fn render_full(site: &Site, chapters: &[ChapterText<'_>]) -> String {
	let prefix = site.url.as_deref().unwrap_or("");
	let mut out = String::new();
	let _ = writeln!(out, "# {}\n", site.title);

	if let Some(description) = site.description.as_deref() {
		let _ = writeln!(out, "> {description}\n");
	}

	for entry in chapters {
		let _ = writeln!(out, "---\n\nSource: {prefix}{}\n", entry.source);
		out.push_str(&entry.markdown);
		out.push('\n');
	}

	out
}
