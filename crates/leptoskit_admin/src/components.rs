//! The pieces every screen is built from: cards, buttons, the mint tab,
//! margin notes, status words, meters, code blocks, and empty states.
//!
//! Interactive pieces carry a `data-testid` the admin's script reads
//! (`workings-tab:<card>`, `note-dismiss:<id>`, `action:<kind>`), so the
//! server renders the whole page and the script only adds behaviour.

use leptoskit::native::Color;
use leptoskit::native::TextSpan;
use leptoskit::prelude::*;

use crate::style;

/// A state word's tone, always shown with an icon and a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
	/// ✓ Healthy, Succeeded, Create.
	Good,
	/// ! Degraded, Waiting, Update.
	Warn,
	/// ✕ Failed, Blocked, Delete, Replace.
	Bad,
	/// • Running, No change, Not checked.
	Info,
}

impl Tone {
	fn icon(self) -> &'static str {
		match self {
			Self::Good => "✓",
			Self::Warn => "!",
			Self::Bad => "✕",
			Self::Info => "•",
		}
	}

	/// The text colour class for this tone.
	#[must_use]
	pub fn text(self) -> &'static str {
		match self {
			Self::Good => "text-[var(--admin-good)]",
			Self::Warn => "text-[var(--admin-warn)]",
			Self::Bad => "text-[var(--admin-bad)]",
			Self::Info => "text-[var(--admin-info)]",
		}
	}
}

/// A state word with its icon: `✓ Healthy`.
pub fn status_word(tone: Tone, word: impl Into<String>) -> Element {
	Text(format!("{} {}", tone.icon(), word.into()))
		.class("text-sm font-semibold")
		.class(tone.text())
}

/// A card with a title row and an optional mint tab.
pub fn card(title: impl Into<String>) -> Card {
	Card {
		title: title.into(),
		aside: None,
		workings: None,
		test_id: None,
		body: Vec::new(),
	}
}

/// A card being built: a title row, then its body.
pub struct Card {
	title: String,
	aside: Option<String>,
	workings: Option<(String, String)>,
	test_id: Option<String>,
	body: Vec<Element>,
}

impl Card {
	/// A muted note beside the title, such as a period: `last 24 h`.
	#[must_use]
	pub fn aside(mut self, aside: impl Into<String>) -> Self {
		self.aside = Some(aside.into());
		self
	}

	/// Gives the card a mint tab that opens Workings drawer `drawer`,
	/// labelled with `source` in Workings mode.
	#[must_use]
	pub fn workings(mut self, drawer: impl Into<String>, source: impl Into<String>) -> Self {
		self.workings = Some((drawer.into(), source.into()));
		self
	}

	/// The card's test identifier.
	#[must_use]
	pub fn test_id(mut self, id: impl Into<String>) -> Self {
		self.test_id = Some(id.into());
		self
	}

	/// Adds content under the title row.
	#[must_use]
	pub fn child(mut self, child: Element) -> Self {
		self.body.push(child);
		self
	}

	/// Adds several children.
	#[must_use]
	pub fn children(mut self, children: impl IntoIterator<Item = Element>) -> Self {
		self.body.extend(children);
		self
	}

	/// The finished card.
	#[must_use]
	pub fn build(self) -> Element {
		let mut header = View()
			.class(style::CARD_HEADER)
			.child(Heading(3, self.title).class(style::CARD_TITLE));

		if let Some(aside) = self.aside {
			header = header.child(Text(aside).class(style::LABEL));
		}

		if let Some((drawer, source)) = &self.workings {
			header = header
				.child(View().class("grow"))
				.child(mint_tab(drawer, source));
		}

		let mut card = View().class(style::CARD).child(header).children(self.body);

		if let Some((drawer, _)) = self.workings {
			card = card.test_id(format!("card:{drawer}"));
		}

		if let Some(id) = self.test_id {
			card = card.test_id(id);
		}

		card
	}
}

/// The mint tab: the logo's mint piece and `Workings`, with the card's
/// source beside it in Workings mode.
pub fn mint_tab(drawer: &str, source: &str) -> Element {
	Pressable(format!("Show workings: {source}"))
		.class(style::MINT_TAB)
		.test_id(format!("workings-tab:{drawer}"))
		.child(View().class(style::MINT_PIECE).test_id("mint-piece"))
		.child(Text("Workings"))
		.child(
			Text(source)
				.class(style::MINT_SOURCE)
				.test_id("mint-source"),
		)
}

/// What a button looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	/// Ink fill on an ember-warm offset block.
	Primary,
	/// Surface with a strong border.
	Secondary,
	/// Text only.
	Quiet,
	/// Bad text and border.
	Danger,
}

impl Kind {
	pub fn class_name(self) -> &'static str {
		match self {
			Self::Primary => style::BUTTON_PRIMARY,
			Self::Secondary => style::BUTTON_SECONDARY,
			Self::Quiet => style::BUTTON_QUIET,
			Self::Danger => style::BUTTON_DANGER,
		}
	}
}

/// A link that looks like a button.
pub fn link_button(kind: Kind, label: &str, href: impl Into<String>) -> Element {
	let link = Pressable(label)
		.href(href)
		.class(kind.class_name())
		.child(Text(label));

	if kind == Kind::Primary {
		raised(link)
	} else {
		link
	}
}

/// A plain text link.
pub fn link(label: impl Into<String>, href: impl Into<String>) -> Element {
	let label = label.into();
	Pressable(label.clone())
		.href(href)
		.class("cursor-pointer")
		.child(Text(label).class("text-sm font-semibold text-[var(--admin-mint-text)]"))
}

/// Puts a control on the brand's hard offset block, the elevation that
/// replaces shadows.
pub fn raised(control: Element) -> Element {
	View()
		.class("relative self-start")
		.child(View().class(style::BUTTON_PRIMARY_BLOCK).aria_hidden(true))
		.child(control)
}

/// A margin note in the sketch face, with "Got it".
pub fn margin_note(id: &str, text: impl Into<String>) -> Element {
	View()
		.class(style::NOTE)
		.test_id(format!("note:{id}"))
		.child(Text(text.into()).class(style::NOTE_TEXT))
		.child(
			Button("Got it")
				.class(
					"text-xs font-semibold text-[var(--admin-mint-text)] cursor-pointer \
					 hover:opacity-70",
				)
				.test_id(format!("note-dismiss:{id}")),
		)
}

/// The environment pill: the one place ember fills outside data.
pub fn env_pill(environment: crate::Environment) -> Element {
	let (label, class) = match environment {
		crate::Environment::Production => {
			(
				"production",
				"bg-[var(--admin-ember)] text-white border border-[var(--admin-ember)]",
			)
		}
		crate::Environment::Staging => {
			(
				"staging",
				"border border-[var(--admin-ember)] text-[var(--admin-ember)]",
			)
		}
		crate::Environment::Development => {
			(
				"development",
				"border border-[var(--admin-ink-3)] bg-[var(--admin-sunken)] \
				 text-[var(--admin-ink-2)]",
			)
		}
	};
	Text(label)
		.class("rounded-full px-2.5 py-0.5 text-xs font-semibold")
		.class(class)
		.test_id("env-pill")
}

/// A horizontal meter: used in ink, a threshold tick at 80%, used and
/// total as text. Past the threshold the text gains `!` and turns warn.
pub fn meter(label: &str, used: f64, total: f64, used_text: &str) -> Element {
	let fraction = if total > 0.0 {
		(used / total).clamp(0.0, 1.0)
	} else {
		0.0
	};
	let over = fraction >= 0.8;
	let value = if over {
		Text(format!("! {used_text}"))
			.class("text-sm font-semibold tabular-nums text-[var(--admin-warn)]")
	} else {
		Text(used_text.to_owned()).class("text-sm tabular-nums text-[var(--admin-ink)]")
	};
	View()
		.class("flex-col gap-1.5")
		.child(
			View()
				.class("flex-row items-end justify-between gap-3")
				.child(Text(label.to_owned()).class(style::SECONDARY))
				.child(value),
		)
		.child(bar(fraction, Some(0.8)))
}

/// A bar in `--admin-sunken` with the used part in ink.
pub fn bar(fraction: f64, threshold: Option<f64>) -> Element {
	let percent = (fraction.clamp(0.0, 1.0) * 1000.0).round() / 10.0;
	let mut track = View()
		.class("relative h-2.5 w-full rounded-full bg-[var(--admin-sunken)] overflow-hidden")
		.aria_hidden(true)
		.child(
			View()
				.class("absolute inset-y-0 left-0 rounded-full bg-[var(--admin-ink)]")
				.class(format!("w-[{percent}%]")),
		);

	if let Some(threshold) = threshold {
		let at = (threshold * 1000.0).round() / 10.0;
		track = track.child(
			View()
				.class("absolute inset-y-0 w-0.5 bg-[var(--admin-ink-3)]")
				.class(format!("left-[{at}%]")),
		);
	}

	track
}

/// One line of a code block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
	/// The line's text, without a trailing newline.
	pub text: String,
	/// How the line stands out.
	pub mark: Mark,
	/// The configuration key this line declares, for linked highlighting.
	pub key: Option<String>,
}

/// How a code line stands out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
	/// An ordinary line.
	Plain,
	/// A line a change adds.
	Added,
	/// A line a change removes.
	Removed,
}

impl Line {
	/// An ordinary line.
	pub fn plain(text: impl Into<String>) -> Self {
		Self {
			text: text.into(),
			mark: Mark::Plain,
			key: None,
		}
	}

	/// A marked line.
	pub fn marked(text: impl Into<String>, mark: Mark) -> Self {
		Self {
			text: text.into(),
			mark,
			key: None,
		}
	}

	/// The configuration key this line declares.
	#[must_use]
	pub fn key(mut self, key: impl Into<String>) -> Self {
		self.key = Some(key.into());
		self
	}
}

/// The syntax colours on the ink underside, from the prototype.
const KEY: Color = Color::rgba(0x9e, 0xc1, 0xff, 0xff);
const STRING: Color = Color::rgba(0xf6, 0xd5, 0x8a, 0xff);
const NUMBER: Color = Color::rgba(0xff, 0x9e, 0x7a, 0xff);
const COMMENT: Color = Color::rgba(0x8a, 0x80, 0x96, 0xff);
const PLAIN: Color = Color::rgba(0xf4, 0xef, 0xe8, 0xff);
const ADDED: Color = Color::rgba(0x5f, 0xd0, 0x8a, 0xff);
const REMOVED: Color = Color::rgba(0xff, 0x8a, 0x9e, 0xff);

/// Colours one line of TOML, JSON, or a shell command: section headers and
/// keys, strings, numbers, and comments.
fn highlight(text: &str, mark: Mark) -> Vec<TextSpan> {
	let mut spans = Vec::new();
	let (prefix, rest) = match mark {
		Mark::Added => ("+", text.strip_prefix('+').unwrap_or(text)),
		Mark::Removed => ("-", text.strip_prefix('-').unwrap_or(text)),
		Mark::Plain => ("", text),
	};

	if !prefix.is_empty() {
		spans.push(TextSpan::new(prefix).color(if mark == Mark::Added { ADDED } else { REMOVED }));
	}

	let trimmed = rest.trim_start();

	if trimmed.starts_with('#') {
		spans.push(TextSpan::new(rest).color(COMMENT));
		return spans;
	}

	if trimmed.starts_with('[') && trimmed.ends_with(']') {
		spans.push(TextSpan::new(rest).color(KEY));
		return spans;
	}

	let mut word = String::new();
	let mut in_string = false;
	let flush = |word: &mut String, spans: &mut Vec<TextSpan>, color: Color| {
		if !word.is_empty() {
			spans.push(TextSpan::new(std::mem::take(word)).color(color));
		}
	};

	for character in rest.chars() {
		if in_string {
			word.push(character);

			if character == '"' {
				in_string = false;
				flush(&mut word, &mut spans, STRING);
			}

			continue;
		}

		if character == '"' {
			let color = classify(&word);
			flush(&mut word, &mut spans, color);
			word.push(character);
			in_string = true;
			continue;
		}

		if matches!(character, '=' | ':' | ',' | '{' | '}' | '[' | ']' | ' ') {
			let color = classify(&word);
			flush(&mut word, &mut spans, color);
			spans.push(TextSpan::new(character.to_string()).color(PLAIN));
			continue;
		}

		word.push(character);
	}

	let color = if in_string { STRING } else { classify(&word) };
	flush(&mut word, &mut spans, color);
	spans
}

fn classify(word: &str) -> Color {
	if word.parse::<f64>().is_ok() || matches!(word, "true" | "false") {
		NUMBER
	} else if word.is_empty() {
		PLAIN
	} else {
		KEY
	}
}

/// A code block on the ink underside: line numbers, the change marks, a
/// per-line wash, and **Copy**. `copy_as` names what Copy gives, such as
/// `TOML`.
pub fn code_block(lines: &[Line], copy_as: &str) -> Element {
	let mut block = View().class(style::CODE_BLOCK).test_id("code");

	for (index, line) in lines.iter().enumerate() {
		let wash = match line.mark {
			Mark::Added => "bg-[#5fd08a]/15",
			Mark::Removed => "bg-[#ff8a9e]/15",
			Mark::Plain => "",
		};
		let mut row = View()
			.class("flex-row gap-3 px-3")
			.class(wash)
			.test_id(match &line.key {
				Some(key) => format!("code-line:{key}"),
				None => "code-line".to_owned(),
			})
			.child(
				Text((index + 1).to_string())
					.class(
						"w-5 shrink-0 text-end font-[var(--admin-font-mono)] text-[12px] \
						 leading-relaxed text-[#6e6578] tabular-nums",
					)
					.aria_hidden(true),
			)
			.child(
				Text("")
					.spans(highlight(&line.text, line.mark))
					.class(
						"min-w-0 flex-1 font-[var(--admin-font-mono)] text-[12.5px] \
						 leading-relaxed text-[#f4efe8]",
					)
					.test_id("code-text"),
			);

		if line.mark != Mark::Plain {
			row = row.aria_description(if line.mark == Mark::Added {
				"added"
			} else {
				"removed"
			});
		}

		block = block.child(row);
	}

	block.child(
		Button("Copy")
			.aria_label(format!("Copy as {copy_as}"))
			.class(
				"absolute right-1.5 top-1.5 rounded-md border border-[#4a3f58] bg-[#1d1726] px-2 \
				 py-0.5 text-[11px] font-semibold text-[#c9c0d0] cursor-pointer hover:text-white",
			)
			.test_id("copy"),
	)
}

/// A code block on paper, for code a page asks you to add.
pub fn paper_code(text: &str, copy_as: &str) -> Element {
	let mut block = View().class(style::CODE_BLOCK_PAPER).test_id("code");

	for line in text.lines() {
		block = block.child(
			View().class("flex-row px-3").test_id("code-line").child(
				Text(line.to_owned())
					.class(
						"min-w-0 flex-1 font-[var(--admin-font-mono)] text-[12.5px] \
						 leading-relaxed text-[var(--admin-ink)]",
					)
					.test_id("code-text"),
			),
		);
	}

	block.child(
		Button("Copy")
			.aria_label(format!("Copy as {copy_as}"))
			.class(
				"absolute right-1.5 top-1.5 rounded-md border border-[var(--admin-line-strong)] \
				 bg-[var(--admin-surface)] px-2 py-0.5 text-[11px] font-semibold \
				 text-[var(--admin-ink-2)] cursor-pointer hover:text-[var(--admin-ink)]",
			)
			.test_id("copy"),
	)
}

/// A sentence whose `backticked` commands are set apart, the way the
/// confirm sheet shows them.
pub fn prose(text: &str) -> Element {
	Text("").spans(text.split('`').enumerate().map(|(index, part)| {
		if index % 2 == 0 {
			TextSpan::new(part)
		} else {
			TextSpan::new(part).weight(leptoskit::native::FontWeight::Semibold)
		}
	}))
}

/// Inline code on paper.
pub fn code(text: impl Into<String>) -> Element {
	Text(text.into()).class(style::CODE)
}

/// A label and a value stacked, as the "Now serving" strip shows them.
pub fn fact(label: &str, value: Element) -> Element {
	View()
		.class("flex-col gap-1 min-w-0")
		.child(Text(label.to_owned()).class(style::MUTED))
		.child(value)
}

/// A tab row; the selected tab has an ink underline.
pub fn tabs(items: &[(String, String, Option<String>)], selected: usize) -> Element {
	let mut row = View()
		.class("flex-row flex-wrap gap-1 border-b border-[var(--admin-line)] overflow-hidden")
		.role(leptoskit::native::Role::Navigation)
		.aria_label("Project sections");

	for (index, (label, href, badge)) in items.iter().enumerate() {
		let on = index == selected;
		let mut content = View()
			.class("flex-row items-center gap-1.5 px-3 py-2 border-b-2")
			.class(if on {
				"border-[var(--admin-ink)]"
			} else {
				"border-transparent"
			})
			.child(Text(label.clone()).class(if on {
				"text-sm font-semibold text-[var(--admin-ink)]"
			} else {
				"text-sm font-semibold text-[var(--admin-ink-3)]"
			}));

		if let Some(badge) = badge {
			content = content.child(Text(badge.clone()).class(
				"rounded-full border border-[var(--admin-line-strong)] px-1.5 text-xs \
				 font-semibold tabular-nums text-[var(--admin-ink-2)]",
			));
		}

		let mut tab = Pressable(label.clone())
			.href(href.clone())
			.class("cursor-pointer hover:bg-[var(--admin-sunken)] rounded-lg")
			.child(content);

		if on {
			tab = tab.aria_current(leptoskit::native::Current::Page);
		}

		row = row.child(tab);
	}

	row
}

/// A teaching empty state: what's missing, the code that fills it, and
/// what to do next.
pub fn empty_state(
	title: &str,
	explanation: &str,
	code_text: Option<(&str, &str)>,
	footnote: &str,
) -> Element {
	let mut body = View()
		.class(style::CARD)
		.class("gap-4")
		.test_id("empty-state")
		.child(
			View()
				.class("flex-row items-center gap-2")
				.child(
					View()
						.class("h-2.5 w-2.5 rounded-full bg-[var(--admin-ink-3)] animate-pulse")
						.aria_hidden(true),
				)
				.child(Heading(2, title.to_owned()).class(style::CARD_TITLE)),
		)
		.child(Text(explanation.to_owned()).class(style::SECONDARY));

	if let Some((code, copy_as)) = code_text {
		body = body.child(paper_code(code, copy_as));
	}

	body.child(Text(footnote.to_owned()).class(style::MUTED))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn highlighting_keeps_the_text() {
		for (text, mark) in [
			("[infra]", Mark::Plain),
			("name = \"notes-production\"", Mark::Plain),
			("+instances = 2", Mark::Added),
			("-instances = 1", Mark::Removed),
			("\"count\": 2,", Mark::Plain),
			("# a comment", Mark::Plain),
		] {
			let joined: String = highlight(text, mark)
				.iter()
				.map(|span| span.text.as_str())
				.collect();
			assert_eq!(joined, text);
		}
	}
}
