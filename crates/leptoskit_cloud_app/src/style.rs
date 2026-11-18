//! The console's tokens and the classes every screen is built from
//! (`docs/agents/cloud-design.md` §4).
//!
//! Screens never spell a colour: they use these constants, which read the
//! `--cloud-*` custom properties. In the browser [`tokens_css`] defines each
//! property once for the light appearance and once for the dark one,
//! following leptoskit's rule: the device's setting, unless the root element
//! carries `data-lk-appearance`. Off the web, and in tests, [`install`] puts
//! one appearance's values in leptoskit's [`Theme`], which is all a native
//! surface reads.
//!
//! The constants that carry over from `leptoskit_admin::style` keep its names
//! and classes with `--admin-` renamed to `--cloud-`, so moving both into a
//! shared crate later is mechanical.

use std::fmt::Write as _;

use leptoskit::native::Color;
use leptoskit::prelude::Theme;

/// One colour token: its name and its light and dark values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
	/// The property's name after `--cloud-`.
	pub name: &'static str,
	/// The value in the light appearance.
	pub light: &'static str,
	/// The value in the dark appearance.
	pub dark: &'static str,
}

const fn token(name: &'static str, light: &'static str, dark: &'static str) -> Token {
	Token { name, light, dark }
}

/// Every colour token, from `docs/agents/cloud-design.md` §4.
pub const TOKENS: &[Token] = &[
	token("paper", "#faf6f0", "#16111f"),
	token("surface", "#fffdf8", "#1d1726"),
	token("sunken", "#f3ede4", "#251e30"),
	token("line", "#e8e0d4", "#30283c"),
	token("line-strong", "#cdbfad", "#4a3f58"),
	token("control-line", "#8a7f96", "#827591"),
	token("ink", "#16111f", "#f4efe8"),
	token("ink-2", "#4d4558", "#c9c0d0"),
	token("ink-3", "#6e6578", "#9d93a8"),
	token("ember", "#e5304f", "#ff5a73"),
	token("ember-text", "#c42a47", "#ff5a73"),
	token("ember-fill", "#c42a47", "#ff5a73"),
	token("on-ember", "#ffffff", "#16111f"),
	token("ember-warm", "#ff7a45", "#ff8a5c"),
	token("mint", "#14bf9b", "#5ef2c2"),
	token("mint-text", "#0a6e59", "#5ef2c2"),
	token("mint-wash", "#e3f6ee", "#11302a"),
	token("underside", "#16111f", "#0f0b16"),
	token("underside-line", "#30283c", "#2a2236"),
	token("button", "#16111f", "#f4efe8"),
	token("button-text", "#faf6f0", "#16111f"),
	token("sheet", "#fffaf0", "#221b2c"),
	token("rule", "#efe4d4", "#2c2437"),
	token("block", "#16111f", "#4a3f58"),
	token("pen", "#27348b", "#a9b4ff"),
	token("good", "#1f7a45", "#5fd08a"),
	token("warn", "#9a5b00", "#e3b341"),
	token("bad", "#b8233f", "#ff8a9e"),
	token("info", "#4d4558", "#c9c0d0"),
];

/// The faces: the system sans for reading, Skribble Mono for code and
/// stamps, and Skribble Noto Sketch for the hand that signs.
const FONTS: &str = "--cloud-font-sans:ui-sans-serif,system-ui,-apple-system,\"Segoe \
                     UI\",Roboto,\"Helvetica Neue\",sans-serif;--cloud-font-mono:\"Skribble \
                     Mono\",ui-monospace,\"SF Mono\",Menlo,monospace;--cloud-font-hand:\"Skribble \
                     Sketch\",\"Comic Neue\",ui-rounded,cursive;";

/// Where the server serves the two faces, which [`fonts_css`] names.
pub const FONT_PATH: &str = "/_leptoskit/fonts";

/// The mono face's file name under [`FONT_PATH`].
pub const MONO_FONT: &str = "skribble-mono.woff2";

/// The hand face's file name under [`FONT_PATH`].
pub const HAND_FONT: &str = "skribble-sketch.woff2";

/// Which appearance's values to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
	/// Paper and ink.
	Light,
	/// Ink and paper.
	Dark,
}

impl Token {
	/// The value in `appearance`.
	#[must_use]
	pub const fn value(&self, appearance: Appearance) -> &'static str {
		match appearance {
			Appearance::Light => self.light,
			Appearance::Dark => self.dark,
		}
	}
}

fn declarations(appearance: Appearance) -> String {
	TOKENS.iter().fold(String::new(), |mut css, token| {
		let _ = write!(css, "--cloud-{}:{};", token.name, token.value(appearance));
		css
	})
}

/// The custom properties for both appearances, for the document's head.
#[must_use]
pub fn tokens_css() -> String {
	let light = declarations(Appearance::Light);
	let dark = declarations(Appearance::Dark);
	format!(
		":root{{{FONTS}{light}color-scheme:light}}@media \
		 (prefers-color-scheme:dark){{:root:not([data-lk-appearance=light]){{{dark}color-scheme:\
		 dark}}}}:root[data-lk-appearance=dark]{{{dark}color-scheme:dark}}"
	)
}

/// The two Skribble faces, served by the console's server.
#[must_use]
pub fn fonts_css() -> String {
	format!(
		"@font-face{{font-family:\"Skribble Mono\";src:url({FONT_PATH}/{MONO_FONT}) \
		 format(\"woff2\");font-display:swap}}@font-face{{font-family:\"Skribble \
		 Sketch\";src:url({FONT_PATH}/{HAND_FONT}) format(\"woff2\");font-display:swap}}"
	)
}

/// Everything the document's head carries: the tokens, the faces, and the
/// page itself on paper, so nothing flashes white before the console draws.
#[must_use]
pub fn document_css() -> String {
	format!(
		"{}{}html,body{{margin:0;background:var(--cloud-paper);color:var(--cloud-ink);font-family:\
		 var(--cloud-font-sans)}}",
		tokens_css(),
		fonts_css()
	)
}

/// Puts `appearance`'s values in leptoskit's [`Theme`], for a native surface
/// or a test. The browser reads [`tokens_css`] instead, which follows the
/// device's setting.
pub fn install(appearance: Appearance) {
	for token in TOKENS {
		if let Some(color) = Color::hex(token.value(appearance)) {
			Theme::set_color(format!("--cloud-{}", token.name), color);
		}
	}
}

// Carried over from the admin, renamed.

/// The page behind everything.
pub const PAGE: &str = "bg-[var(--cloud-paper)] text-[var(--cloud-ink)]";
/// A card: surface, hairline border, rounded, padded.
pub const CARD: &str = "flex-col gap-3 rounded-xl border border-[var(--cloud-line)] \
                        bg-[var(--cloud-surface)] p-4 md:p-5";
/// A card's header row, which wraps so a long Workings label drops a line.
pub const CARD_HEADER: &str = "flex-row flex-wrap items-center gap-2";
/// A card's title.
pub const CARD_TITLE: &str = "text-base font-semibold text-[var(--cloud-ink)]";
/// A page title.
pub const PAGE_TITLE: &str = "text-2xl font-semibold leading-tight text-[var(--cloud-ink)]";
/// Body text.
pub const BODY: &str = "text-sm leading-normal text-[var(--cloud-ink)]";
/// Secondary text.
pub const SECONDARY: &str = "text-sm leading-normal text-[var(--cloud-ink-2)]";
/// Muted text: timestamps, captions, fine print.
pub const MUTED: &str = "text-sm leading-normal text-[var(--cloud-ink-3)]";
/// A small label: timestamps, field labels.
pub const LABEL: &str = "text-xs font-medium text-[var(--cloud-ink-3)]";
/// A group label in capitals, as the rail uses.
pub const GROUP_LABEL: &str = "text-xs font-semibold tracking-wider text-[var(--cloud-ink-3)]";
/// A hero number.
pub const HERO_NUMBER: &str =
	"text-3xl font-semibold leading-none tabular-nums text-[var(--cloud-ink)]";
/// Inline code on paper.
pub const CODE: &str =
	"font-[var(--cloud-font-mono)] text-[13px] leading-relaxed text-[var(--cloud-ink)]";

/// The primary button: ink fill, paper text. Sits on
/// [`BUTTON_PRIMARY_BLOCK`].
pub const BUTTON_PRIMARY: &str =
	"relative min-h-11 flex-row items-center justify-center gap-2 rounded-lg border \
	 border-[var(--cloud-button)] bg-[var(--cloud-button)] px-4 py-2 text-sm font-semibold \
	 text-[var(--cloud-button-text)] cursor-pointer transition-transform duration-75 \
	 active:translate-x-1 active:translate-y-1";
/// The hard offset block behind a primary button.
pub const BUTTON_PRIMARY_BLOCK: &str =
	"absolute inset-0 translate-x-1 translate-y-1 rounded-lg bg-[var(--cloud-ember-warm)]";
/// A secondary button: surface with a control-contrast hairline.
pub const BUTTON_SECONDARY: &str =
	"min-h-11 flex-row items-center justify-center gap-2 rounded-lg border \
	 border-[var(--cloud-control-line)] bg-[var(--cloud-surface)] px-4 py-2 text-sm font-semibold \
	 text-[var(--cloud-ink)] cursor-pointer hover:bg-[var(--cloud-sunken)]";
/// A quiet button: text only.
pub const BUTTON_QUIET: &str = "min-h-11 flex-row items-center gap-2 rounded-lg px-3 py-2 text-sm \
                                font-semibold text-[var(--cloud-ink-2)] cursor-pointer \
                                hover:bg-[var(--cloud-sunken)]";
/// A danger button: bad text and border.
pub const BUTTON_DANGER: &str = "min-h-11 flex-row items-center justify-center gap-2 rounded-lg \
                                 border border-[var(--cloud-bad)] bg-[var(--cloud-surface)] px-4 \
                                 py-2 text-sm font-semibold text-[var(--cloud-bad)] cursor-pointer";
/// A quiet danger button: bad text only, such as "Not me, deny".
pub const BUTTON_DANGER_QUIET: &str = "min-h-11 flex-row items-center gap-2 rounded-lg px-3 py-2 \
                                       text-sm font-semibold text-[var(--cloud-bad)] \
                                       cursor-pointer hover:bg-[var(--cloud-sunken)]";

/// The mint tab at rest: the door to a card's back.
pub const MINT_TAB: &str = "min-h-11 flex-row items-center gap-1.5 rounded-full border \
                            border-[var(--cloud-mint)]/50 bg-[var(--cloud-mint-wash)] py-0.5 \
                            pl-1.5 pr-2.5 text-xs font-semibold text-[var(--cloud-mint-text)] \
                            cursor-pointer overflow-hidden max-w-full";
/// The tilted mint piece inside a tab.
pub const MINT_PIECE: &str = "h-3 w-3 shrink-0 rounded-[3px] bg-[var(--cloud-mint)] -rotate-12";

/// The back of a card: always ink.
pub const UNDERSIDE: &str = "flex-col gap-3 rounded-xl bg-[var(--cloud-underside)] p-4 md:p-5";
/// Text on the back of a card.
pub const UNDERSIDE_TEXT: &str = "text-sm leading-normal text-[#f4efe8]";
/// A code block on the back of a card.
pub const CODE_BLOCK: &str = "relative flex-col rounded-[10px] border \
                              border-[var(--cloud-underside-line)] bg-[#0b0811] py-2.5 \
                              overflow-hidden";
/// A code block on paper.
pub const CODE_BLOCK_PAPER: &str = "relative flex-col rounded-[10px] border \
                                    border-[var(--cloud-line)] bg-[var(--cloud-sunken)] py-2.5 \
                                    overflow-hidden";

/// A pill-shaped chip.
pub const CHIP: &str = "min-h-11 flex-row items-center gap-1.5 rounded-full px-1 text-xs \
                        font-medium text-[var(--cloud-ink-2)]";
/// The visible pill inside a [`CHIP`]'s target.
pub const CHIP_FACE: &str = "flex-row items-center gap-1.5 rounded-full border \
                             border-[var(--cloud-line-strong)] bg-[var(--cloud-surface)] px-2.5 \
                             py-0.5";

// New in the cloud.

/// The whole console: the top bar, the rail and content, the tab bar.
pub const SHELL: &str = "h-screen flex-col bg-[var(--cloud-paper)] text-[var(--cloud-ink)]";
/// The top bar.
pub const TOP_BAR: &str = "h-12 sm:h-14 flex-row items-center gap-3 px-4 bg-[var(--cloud-surface)]";
/// A 1px rule across the page.
pub const RULE_ACROSS: &str = "h-px w-full bg-[var(--cloud-line)]";
/// A 1px rule down the page.
pub const RULE_DOWN: &str = "w-px h-full bg-[var(--cloud-line)]";
/// The rail: icons on tablets, full width on desktops, hidden on phones.
pub const RAIL: &str =
	"hidden sm:flex w-16 lg:w-60 flex-col gap-1 px-2 py-3 bg-[var(--cloud-surface)]";
/// A rail entry.
pub const RAIL_ITEM: &str = "min-h-11 flex-row items-center gap-3 rounded-lg px-3 text-sm \
                             text-[var(--cloud-ink-2)] hover:bg-[var(--cloud-sunken)]";
/// The rail entry for the page you're on: weight and fill, not colour alone.
pub const RAIL_ITEM_CURRENT: &str = "min-h-11 flex-row items-center gap-3 rounded-lg px-3 text-sm \
                                     bg-[var(--cloud-sunken)] font-semibold \
                                     text-[var(--cloud-ink)]";
/// A rail entry's words, hidden on tablets where the rail is icons.
pub const RAIL_WORDS: &str = "hidden lg:flex flex-1 truncate";
/// The phones' tab bar.
pub const TAB_BAR: &str =
	"flex sm:hidden h-16 flex-row items-stretch bg-[var(--cloud-surface)] pb-safe";
/// A tab.
pub const TAB: &str = "flex-1 flex-col items-center justify-center gap-0.5 text-[11px] \
                       font-medium text-[var(--cloud-ink-3)]";
/// The tab for the page you're on.
pub const TAB_CURRENT: &str = "flex-1 flex-col items-center justify-center gap-0.5 text-[11px] \
                               font-semibold text-[var(--cloud-ink)]";
/// The 2px ink bar above the current tab's icon.
pub const TAB_BAR_MARK: &str = "h-0.5 w-6 rounded-full bg-[var(--cloud-ink)]";
/// The same space above the other tabs' icons, so nothing shifts.
pub const TAB_BAR_SPACE: &str = "h-0.5 w-6";
/// The content column.
pub const CONTENT: &str = "flex-1 px-4 md:px-6 xl:px-8 py-6 gap-8 max-w-[1200px] w-full";
/// A text field.
pub const INPUT: &str = "min-h-11 rounded-lg border border-[var(--cloud-control-line)] \
                         bg-[var(--cloud-sunken)] px-3 text-sm text-[var(--cloud-ink)]";
/// A text field whose value is wrong.
pub const INPUT_ERROR: &str = "min-h-11 rounded-lg border-2 border-[var(--cloud-bad)] \
                               bg-[var(--cloud-sunken)] px-3 text-sm text-[var(--cloud-ink)]";
/// The production pill.
pub const PILL_PRODUCTION: &str = "rounded-full px-2 py-0.5 text-xs font-semibold \
                                   bg-[var(--cloud-ember-fill)] text-[var(--cloud-on-ember)]";
/// The staging pill.
pub const PILL_STAGING: &str = "rounded-full px-2 py-0.5 text-xs font-semibold border \
                                border-[var(--cloud-ember-text)] text-[var(--cloud-ember-text)]";
/// A preview environment's pill.
pub const PILL_PREVIEW: &str = "rounded-full px-2 py-0.5 text-xs font-semibold \
                                bg-[var(--cloud-sunken)] text-[var(--cloud-ink-2)]";
/// A margin note: mint wash with a 3px mint pencil line as its first child.
pub const NOTE: &str =
	"flex-row items-stretch gap-3 rounded-[10px] bg-[var(--cloud-mint-wash)] p-3";
/// A margin note's pencil line.
pub const NOTE_LINE: &str = "w-[3px] rounded-full bg-[var(--cloud-mint)]";
/// The hand inside a margin note.
pub const NOTE_TEXT: &str =
	"flex-1 font-[var(--cloud-font-hand)] text-[16px] leading-snug text-[var(--cloud-ink-2)]";
/// A placeholder while something loads. It never shimmers.
pub const SKELETON: &str = "rounded-lg bg-[var(--cloud-sunken)]";
/// The focus ring's wrapper while its control is focused.
pub const FOCUS_RING: &str = "rounded-[10px] border-2 border-[var(--cloud-ember)]";
/// The focus ring's wrapper otherwise: the same width, so nothing shifts.
pub const FOCUS_RING_OFF: &str = "rounded-[10px] border-2 border-transparent";
/// The smallest a pressable gets.
pub const TARGET: &str = "min-h-11 min-w-11 items-center justify-center";
/// First-run display: the hand, large.
pub const DISPLAY: &str = "font-[var(--cloud-font-hand)] text-[28px] sm:text-[36px] \
                           leading-[1.15] text-[var(--cloud-ink)]";
/// A plain text link.
pub const LINK: &str = "text-sm font-semibold text-[var(--cloud-mint-text)]";
/// What went wrong with a field, under it.
pub const FIELD_ERROR: &str = "text-sm font-medium text-[var(--cloud-bad)]";

// The sheet (§9.2).

/// The page a sheet sits on: the paper, centred.
pub const DESK: &str = "min-h-screen w-full items-center bg-[var(--cloud-paper)] px-4 py-8 \
                        sm:py-16 text-[var(--cloud-ink)]";
/// The sheet's frame: the paper and its offset block.
pub const SHEET_FRAME: &str = "relative w-full max-w-md";
/// A sheet of paper.
pub const SHEET: &str = "relative flex-col gap-5 rounded-[6px] border \
                         border-[var(--cloud-line-strong)] bg-[var(--cloud-sheet)] px-5 py-6 \
                         md:px-8 md:py-8";
/// The hard offset block under a sheet.
pub const SHEET_BLOCK: &str =
	"absolute inset-0 translate-x-1.5 translate-y-1.5 rounded-[6px] bg-[var(--cloud-block)]";
/// A ruled line across a sheet.
pub const SHEET_RULE: &str = "h-px w-full bg-[var(--cloud-rule)]";
/// A stamp: mono capitals in a tilted box. Add a tone and a rotation.
pub const STAMP: &str = "flex-row items-center self-start rounded-[4px] border-2 px-1.5 py-0.5";
/// A stamp's words.
pub const STAMP_TEXT: &str =
	"font-[var(--cloud-font-mono)] text-[11px] font-semibold tracking-wider leading-none";
/// A sheet's large stamp's words.
pub const STAMP_TEXT_LARGE: &str =
	"font-[var(--cloud-font-mono)] text-[20px] font-semibold tracking-wider leading-none";
/// The sheet's large stamp box.
pub const STAMP_LARGE: &str = "flex-row items-center self-start rounded-[4px] border-2 px-3 py-2";
/// A signature, in pen.
pub const SIGNATURE: &str =
	"font-[var(--cloud-font-hand)] text-[30px] leading-none text-[var(--cloud-pen)]";

/// A stamp's resting rotation, picked from `seed` so it never jiggles
/// between renders.
#[must_use]
pub fn stamp_rotation(seed: &str) -> &'static str {
	const ROTATIONS: [&str; 5] = [
		"-rotate-6",
		"-rotate-3",
		"-rotate-2",
		"rotate-2",
		"rotate-3",
	];
	let sum = seed.bytes().fold(0_usize, |sum, byte| {
		sum.wrapping_mul(31).wrapping_add(usize::from(byte))
	});
	ROTATIONS[sum % ROTATIONS.len()]
}

/// The resting rotation in degrees for the class [`stamp_rotation`] picks.
#[must_use]
pub fn stamp_degrees(class: &str) -> f32 {
	match class {
		"-rotate-6" => -6.0,
		"-rotate-3" => -3.0,
		"-rotate-2" => -2.0,
		"rotate-2" => 2.0,
		"rotate-3" => 3.0,
		_ => 0.0,
	}
}
