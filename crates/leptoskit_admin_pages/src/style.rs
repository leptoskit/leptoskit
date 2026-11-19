//! The admin's tokens and the classes every screen is built from.
//!
//! Screens never spell a colour: they use these constants, which read the
//! `--admin-*` custom properties. [`tokens_css`] defines each property once
//! for the light appearance and once for the dark one, following
//! leptoskit's own rule: the device's setting, unless the root element
//! carries `data-lk-appearance`.

use std::fmt::Write as _;

/// One colour token: its name and its light and dark values.
struct Token {
	name: &'static str,
	light: &'static str,
	dark: &'static str,
}

const fn token(name: &'static str, light: &'static str, dark: &'static str) -> Token {
	Token { name, light, dark }
}

/// Every colour token, from `docs/agents/admin-design.md` §4.
const TOKENS: &[Token] = &[
	token("paper", "#faf6f0", "#16111f"),
	token("surface", "#fffdf8", "#1d1726"),
	token("sunken", "#f3ede4", "#251e30"),
	token("line", "#e8e0d4", "#30283c"),
	token("line-strong", "#cdbfad", "#4a3f58"),
	token("ink", "#16111f", "#f4efe8"),
	token("ink-2", "#4d4558", "#c9c0d0"),
	token("ink-3", "#6e6578", "#9d93a8"),
	token("ember", "#e5304f", "#ff5a73"),
	token("ember-warm", "#ff7a45", "#ff8a5c"),
	token("mint", "#14bf9b", "#5ef2c2"),
	token("mint-text", "#0a6e59", "#5ef2c2"),
	token("mint-wash", "#e3f6ee", "#11302a"),
	token("underside", "#16111f", "#0f0b16"),
	token("underside-line", "#30283c", "#2a2236"),
	token("good", "#1f7a45", "#5fd08a"),
	token("warn", "#9a5b00", "#e3b341"),
	token("bad", "#b8233f", "#ff8a9e"),
	token("info", "#4d4558", "#c9c0d0"),
	// The primary button inverts with the theme: ink on paper, paper on ink.
	token("button", "#16111f", "#f4efe8"),
	token("button-text", "#faf6f0", "#16111f"),
	// Chart series, in their fixed order, then the sequential ramp's ends.
	token("series-1", "#e0492b", "#e8512f"),
	token("series-2", "#2f6fd6", "#3f7fe6"),
	token("series-3", "#2a8f3c", "#2f9a4a"),
	token("series-4", "#8a5cd6", "#8f6ee6"),
	token("series-5", "#b57c00", "#c08a12"),
	token("series-6", "#d6528f", "#d65690"),
	// The sequential ramp for the retention grid, one hue, and the text on
	// each step: white from step 4 in light, near-black from step 5 in dark.
	token("seq-1", "#dde8fa", "#1f2c48"),
	token("seq-2", "#b9cff3", "#22396a"),
	token("seq-3", "#8db1ea", "#2a4d8f"),
	token("seq-4", "#5b8fe0", "#3a66b8"),
	token("seq-5", "#2f6fd6", "#5b8ae6"),
	token("seq-6", "#1f53a8", "#8fb2f2"),
	token("seq-text-1", "#16111f", "#f4efe8"),
	token("seq-text-2", "#16111f", "#f4efe8"),
	token("seq-text-3", "#16111f", "#f4efe8"),
	token("seq-text-4", "#ffffff", "#f4efe8"),
	token("seq-text-5", "#ffffff", "#0f0b16"),
	token("seq-text-6", "#ffffff", "#0f0b16"),
];

/// The faces: the system sans for reading, Skribble Mono for code, and
/// Skribble Noto Sketch for the admin's own notes.
const FONTS: &str = "--admin-font-sans:ui-sans-serif,system-ui,-apple-system,\"Segoe \
                     UI\",Roboto,\"Helvetica Neue\",sans-serif;--admin-font-mono:\"Skribble \
                     Mono\",ui-monospace,\"SF Mono\",Menlo,monospace;--admin-font-note:\"Skribble \
                     Sketch\",\"Comic Neue\",ui-rounded,cursive;";

fn declarations(dark: bool) -> String {
	TOKENS.iter().fold(String::new(), |mut css, token| {
		let _ = write!(
			css,
			"--admin-{}:{};",
			token.name,
			if dark { token.dark } else { token.light }
		);
		css
	})
}

/// The custom properties for both appearances.
#[must_use]
pub fn tokens_css() -> String {
	let light = declarations(false);
	let dark = declarations(true);
	format!(
		":root{{{FONTS}{light}color-scheme:light}}@media \
		 (prefers-color-scheme:dark){{:root:not([data-lk-appearance=light]){{{dark}color-scheme:\
		 dark}}}}:root[data-lk-appearance=dark]{{{dark}color-scheme:dark}}"
	)
}

/// The page behind everything.
pub const PAGE: &str = "bg-[var(--admin-paper)] text-[var(--admin-ink)]";
/// A card: surface, hairline border, rounded, padded.
pub const CARD: &str = "flex-col gap-3 rounded-xl border border-[var(--admin-line)] \
                        bg-[var(--admin-surface)] p-4 md:p-5";
/// A card's header row, which wraps so a long Workings label drops a line.
pub const CARD_HEADER: &str = "flex-row flex-wrap items-center gap-2";
/// A card's title.
pub const CARD_TITLE: &str = "text-base font-semibold text-[var(--admin-ink)]";
/// A page title.
pub const PAGE_TITLE: &str = "text-2xl font-semibold leading-tight text-[var(--admin-ink)]";
/// Body text.
pub const BODY: &str = "text-sm leading-normal text-[var(--admin-ink)]";
/// Secondary text.
pub const SECONDARY: &str = "text-sm leading-normal text-[var(--admin-ink-2)]";
/// Muted text: timestamps, captions, placeholders.
pub const MUTED: &str = "text-sm leading-normal text-[var(--admin-ink-3)]";
/// A small label: axis ticks, table headers, timestamps.
pub const LABEL: &str = "text-xs font-medium text-[var(--admin-ink-3)]";
/// A group label in capitals, as the rail and diff groups use.
pub const GROUP_LABEL: &str = "text-xs font-semibold tracking-wider text-[var(--admin-ink-3)]";
/// A stat tile's hero number.
pub const HERO_NUMBER: &str =
	"text-3xl font-semibold leading-none tabular-nums text-[var(--admin-ink)]";
/// Inline code on paper.
pub const CODE: &str = "font-[var(--admin-font-mono)] text-[13px] text-[var(--admin-ink)]";

/// The primary button: ink fill, paper text. Sits on [`BUTTON_PRIMARY_BLOCK`].
pub const BUTTON_PRIMARY: &str = "relative flex-row items-center justify-center gap-2 rounded-lg \
                                  border border-[var(--admin-button)] bg-[var(--admin-button)] \
                                  px-4 py-2 text-sm font-semibold text-[var(--admin-button-text)] \
                                  cursor-pointer transition-transform duration-75 \
                                  active:translate-x-1 active:translate-y-1";
/// The hard offset block behind a primary button.
pub const BUTTON_PRIMARY_BLOCK: &str =
	"absolute inset-0 translate-x-1 translate-y-1 rounded-lg bg-[var(--admin-ember-warm)]";
/// A secondary button: surface with a strong hairline.
pub const BUTTON_SECONDARY: &str = "flex-row items-center justify-center gap-2 rounded-lg border \
                                    border-[var(--admin-line-strong)] bg-[var(--admin-surface)] \
                                    px-4 py-2 text-sm font-semibold text-[var(--admin-ink)] \
                                    cursor-pointer hover:bg-[var(--admin-sunken)]";
/// A quiet button: text only.
pub const BUTTON_QUIET: &str = "flex-row items-center gap-2 rounded-lg px-3 py-2 text-sm \
                                font-semibold text-[var(--admin-ink-2)] cursor-pointer \
                                hover:bg-[var(--admin-sunken)]";
/// A danger button: bad text and border, filled only inside a confirm.
pub const BUTTON_DANGER: &str = "flex-row items-center justify-center gap-2 rounded-lg border \
                                 border-[var(--admin-bad)] bg-[var(--admin-surface)] px-4 py-2 \
                                 text-sm font-semibold text-[var(--admin-bad)] cursor-pointer";

/// The mint tab at rest: the machinery's door on every card.
pub const MINT_TAB: &str = "flex-row items-center gap-1.5 rounded-full border \
                            border-[var(--admin-mint)]/50 bg-[var(--admin-mint-wash)] py-0.5 \
                            pl-1.5 pr-2.5 text-xs font-semibold text-[var(--admin-mint-text)] \
                            cursor-pointer overflow-hidden max-w-full";
/// The tilted mint piece inside a tab.
pub const MINT_PIECE: &str = "h-3 w-3 shrink-0 rounded-[3px] bg-[var(--admin-mint)] -rotate-12 \
                              transition-transform duration-200";
/// The source label inside a mint tab, shown in Workings mode.
pub const MINT_SOURCE: &str =
	"truncate font-[var(--admin-font-mono)] text-[11.5px] font-medium text-[var(--admin-ink-2)]";

/// The Workings drawer: always ink.
pub const UNDERSIDE: &str = "flex-col bg-[var(--admin-underside)] text-[#f4efe8]";
/// A code block in the drawer.
pub const CODE_BLOCK: &str = "relative flex-col rounded-[10px] border \
                              border-[var(--admin-underside-line)] bg-[#0b0811] py-2.5 \
                              overflow-hidden";
/// A code block on paper.
pub const CODE_BLOCK_PAPER: &str = "relative flex-col rounded-[10px] border \
                                    border-[var(--admin-line)] bg-[var(--admin-sunken)] py-2.5 \
                                    overflow-hidden";

/// A margin note: sketch face on mint wash, with a mint pencil line.
pub const NOTE: &str = "flex-row items-start gap-3 rounded-[10px] border-l-[3px] \
                        border-[var(--admin-mint)] bg-[var(--admin-mint-wash)] px-3 py-2";
/// The handwriting inside a margin note.
pub const NOTE_TEXT: &str =
	"flex-1 font-[var(--admin-font-note)] text-[16px] leading-snug text-[var(--admin-ink-2)]";

/// A pill-shaped chip.
pub const CHIP: &str = "flex-row items-center gap-1.5 rounded-full border \
                        border-[var(--admin-line-strong)] bg-[var(--admin-surface)] px-2.5 py-0.5 \
                        text-xs font-medium text-[var(--admin-ink-2)]";

/// The colour class for a chart series slot, from 1 to 6.
#[must_use]
pub fn series(slot: usize) -> &'static str {
	match slot {
		1 => "text-[var(--admin-series-1)]",
		2 => "text-[var(--admin-series-2)]",
		3 => "text-[var(--admin-series-3)]",
		4 => "text-[var(--admin-series-4)]",
		5 => "text-[var(--admin-series-5)]",
		6 => "text-[var(--admin-series-6)]",
		_ => "text-[var(--admin-ink-3)]",
	}
}

#[cfg(test)]
mod tests {
	use leptoskit::prelude::Tailwind;

	use super::*;

	#[test]
	fn every_class_compiles() {
		for class in [
			PAGE,
			CARD,
			CARD_HEADER,
			CARD_TITLE,
			PAGE_TITLE,
			BODY,
			SECONDARY,
			MUTED,
			LABEL,
			GROUP_LABEL,
			HERO_NUMBER,
			CODE,
			BUTTON_PRIMARY,
			BUTTON_PRIMARY_BLOCK,
			BUTTON_SECONDARY,
			BUTTON_QUIET,
			BUTTON_DANGER,
			MINT_TAB,
			MINT_PIECE,
			MINT_SOURCE,
			UNDERSIDE,
			CODE_BLOCK,
			CODE_BLOCK_PAPER,
			NOTE,
			NOTE_TEXT,
			CHIP,
		] {
			Tailwind::parse(class).unwrap_or_else(|error| panic!("{error}"));
		}

		for slot in 0..=7 {
			Tailwind::parse(series(slot)).unwrap();
		}
	}

	#[test]
	fn tokens_have_both_appearances() {
		let css = tokens_css();
		assert!(css.contains("--admin-paper:#faf6f0;"));
		assert!(css.contains("--admin-paper:#16111f;"));
		assert!(css.contains(":root[data-lk-appearance=dark]"));
	}
}
