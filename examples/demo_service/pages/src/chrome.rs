//! The pieces every page shares: the mark, the demo band, the app bar,
//! the raised button, status messages, and the new-day dialog.

use leptoskit::native::Live;
use leptoskit::native::Role;
use leptoskit::native::TextSpan;
use leptoskit::prelude::*;

use crate::Shell;
use crate::model::countdown_text;

/// The page behind everything: paper, with ink text.
pub(crate) const PAGE: &str =
	"min-h-screen w-full flex-col bg-[var(--admin-paper)] text-[var(--admin-ink)]";

/// The content column every page's body sits in.
pub(crate) const COLUMN: &str = "w-full max-w-[1120px] self-center px-4 md:px-6";

/// The primary button: ink fill, paper text. Sits on [`raised`]'s block.
pub(crate) const BUTTON_PRIMARY: &str =
	"relative min-h-[44px] flex-row items-center justify-center gap-2 rounded-lg border \
	 border-[var(--admin-button)] bg-[var(--admin-button)] px-4 py-2.5 text-[15px] font-semibold \
	 text-[var(--admin-button-text)] cursor-pointer transition-transform duration-75 \
	 active:translate-x-1 active:translate-y-1";

/// A secondary button: surface with a strong hairline.
pub(crate) const BUTTON_SECONDARY: &str =
	"min-h-[44px] flex-row items-center justify-center gap-2 rounded-lg border \
	 border-[var(--admin-line-strong)] bg-[var(--admin-surface)] px-4 py-2.5 text-[15px] \
	 font-semibold text-[var(--admin-ink)] cursor-pointer hover:bg-[var(--admin-sunken)]";

/// A quiet button: text only.
pub(crate) const BUTTON_QUIET: &str = "min-h-[44px] flex-row items-center gap-2 rounded-lg px-3 \
                                       py-2 text-sm font-semibold text-[var(--admin-ink-2)] \
                                       cursor-pointer hover:bg-[var(--admin-sunken)]";

/// Body text, quieter text, and the smallest text.
pub(crate) const SECONDARY: &str = "text-sm leading-normal text-[var(--admin-ink-2)]";
pub(crate) const MUTED: &str = "text-[13px] leading-normal text-[var(--admin-ink-3)]";
pub(crate) const LABEL: &str = "text-xs font-semibold text-[var(--admin-ink-2)]";

/// The leptoskit mark: two ember boxes and the mint piece that arrives
/// over the air, tilted −12°.
pub(crate) fn mark(size: f32) -> Element {
	let unit = size / 28.0;
	let px = |value: f32| format!("{:.1}px", value * unit);
	view! {
		<View class=format!("relative shrink-0 h-[{}] w-[{}]", px(28.0), px(28.0)) aria_hidden=true>
			<View class=format!(
				"absolute rounded-[3px] bg-[var(--admin-ember)] left-0 top-[{}] h-[{}] w-[{}]",
				px(4.0),
				px(24.0),
				px(11.0)
			) />
			<View class=format!(
				"absolute rounded-[3px] bg-[var(--admin-ember-warm)] left-[{}] top-[{}] h-[{}] \
				 w-[{}]",
				px(14.0),
				px(17.0),
				px(11.0),
				px(11.0)
			) />
			<View class=format!(
				"absolute rounded-[3px] bg-[var(--admin-mint)] -rotate-12 left-[{}] top-0 h-[{}] \
				 w-[{}]",
				px(15.0),
				px(11.0),
				px(11.0)
			) />
		</View>
	}
}

/// One paragraph of text in which some runs are bold.
pub(crate) fn rich(class: &str, parts: &[(&str, bool)]) -> Element {
	let spans = parts
		.iter()
		.map(|(text, bold)| {
			if *bold {
				TextSpan::new(*text).bold()
			} else {
				TextSpan::new(*text)
			}
		})
		.collect::<Vec<_>>();
	view! { <Text class=class spans=spans /> }
}

/// The tilted mint piece: the machinery's mark.
pub(crate) fn piece() -> Element {
	view! {
		<View class="h-3 w-3 shrink-0 rounded-[3px] bg-[#5ef2c2] -rotate-12" aria_hidden=true />
	}
}

/// A button on its hard ember-warm offset block.
pub(crate) fn raised(button: Element, full: bool) -> Element {
	let width = if full {
		"relative w-full"
	} else {
		"relative self-start"
	};
	view! {
		<View class=width>
			<View
				class="absolute inset-0 translate-x-1 translate-y-1 rounded-lg \
				       bg-[var(--admin-ember-warm)]"
				aria_hidden=true
			/>
			{button}
		</View>
	}
}

/// How serious a status message is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
	Info,
	Good,
	Warn,
	Bad,
}

impl Tone {
	const fn classes(self) -> &'static str {
		match self {
			Self::Info => {
				"border-[var(--admin-line-strong)] bg-[var(--admin-surface)] \
				 text-[var(--admin-ink-2)]"
			}
			Self::Good => {
				"border-[var(--admin-good)] bg-[var(--admin-sunken)] text-[var(--admin-good)]"
			}
			Self::Warn => {
				"border-[var(--admin-warn)] bg-[var(--admin-sunken)] text-[var(--admin-warn)]"
			}
			Self::Bad => {
				"border-[var(--admin-bad)] bg-[var(--admin-sunken)] text-[var(--admin-bad)]"
			}
		}
	}

	const fn glyph(self) -> &'static str {
		match self {
			Self::Info => "•",
			Self::Good => "✓",
			Self::Warn => "!",
			Self::Bad => "✕",
		}
	}
}

/// A status message: a glyph in the tone's colour, a title, and body
/// text. `children` may add a control under the body. Errors announce at
/// once, everything else politely.
pub(crate) fn status(
	tone: Tone,
	test_id: &str,
	title: &str,
	body: &str,
	children: Vec<Element>,
) -> Element {
	let role = if matches!(tone, Tone::Bad) {
		Role::Alert
	} else {
		Role::Status
	};
	view! {
		<View
			class=format!(
				"flex-row items-start gap-2.5 rounded-[10px] border px-3 py-2.5 {}",
				tone.classes()
			)
			role=role
			test_id=test_id.to_owned()
		>
			<Text
				class="w-4 shrink-0 text-center text-sm font-extrabold"
				aria_hidden=true
				test_id="status-glyph"
			>
				{tone.glyph()}
			</Text>
			<View class="min-w-0 flex-1 flex-col gap-1.5">
				<Text
					class="text-sm font-semibold leading-normal text-[var(--admin-ink)]"
					test_id="status-title"
				>
					{title.to_owned()}
				</Text>
				<Text class=SECONDARY test_id="status-body">{body.to_owned()}</Text>
				{children}
			</View>
		</View>
	}
}

/// The thin ink band on every page: the demo says what it is, when it
/// resets, and what is real.
///
/// The countdown is bound to the page's clock, so one second passing
/// rewrites that text and nothing else. The band is rebuilt only when it
/// opens or closes, or when the last ten minutes begin.
pub(crate) fn band(shell: &Shell) -> Element {
	let late = shell.late.get();
	let open = shell.band_open.get();
	let band_open = shell.band_open;
	let countdown_class = if late {
		"font-semibold text-[var(--admin-warn)]"
	} else {
		"text-[#f4efe8]"
	};
	let seconds = shell.seconds_to_reset();
	let countdown = move || {
		let seconds = seconds();
		format!(
			"{}Resets in {} (00:00 UTC)",
			if crate::model::is_late(seconds) {
				"! "
			} else {
				""
			},
			countdown_text(seconds)
		)
	};
	// Closed, the panel is hidden. A browser with scripts off shows it all
	// the same (`assets/demo.css`), so what is real is never hidden from
	// anyone.
	let panel_class = if open {
		"w-full max-w-[1120px] self-center flex-col gap-4 px-4 pb-5 pt-1 md:flex-row md:gap-6"
	} else {
		"hidden w-full max-w-[1120px] self-center flex-col gap-4 px-4 pb-5 pt-1 md:flex-row \
		 md:gap-6"
	};
	view! {
		<View
			class="w-full flex-col border-b border-[var(--admin-underside-line)] \
			       bg-[var(--admin-underside)] text-[#f4efe8]"
			role=Role::Region
			aria_label="About this demo"
			test_id="band"
		>
			<View class="w-full max-w-[1120px] self-center flex-row flex-wrap items-center gap-x-2.5 \
				       gap-y-1 px-4 py-2 text-[13px]">
				{piece()} <Text class="font-semibold text-[#f4efe8]">"Public demo"</Text>
				<Text class="text-[#9d93a8]" aria_hidden=true>"·"</Text>
				<Text class=countdown_class test_id="band-reset">{countdown}</Text>
				<Text class="text-[#c9c0d0]" test_id="band-late">
					{if late {
						"· anything you do now is cleared then"
					} else {
						""
					}}
				</Text>
				<Text class="max-md:hidden text-[#c9c0d0]">"· Shared with everyone today"</Text>
				<Text class="max-md:hidden text-[#c9c0d0]">"· Infrastructure is simulated"</Text>
				<Button
					class="ml-auto min-h-[28px] flex-row items-center rounded-full border \
					       border-[#5ef2c2]/50 px-3 py-0.5 text-[12.5px] font-semibold \
					       text-[#5ef2c2] cursor-pointer"
					test_id="band-toggle"
					aria_expanded=open
					aria_controls=["band-panel"]
					on:press=move || band_open.update(|open| *open = !*open)
				>
					"What's real?"
				</Button>
			</View>
			<View id="band-panel" test_id="band-panel" class=panel_class>
				{band_section(
					"Real",
					"real",
					&[
						("Sign-in", ": codes, sessions, and cookies"),
						("OpenID Connect", " from Notes to its admin"),
						("Roles", ": Ada has admin, Grace doesn't"),
						("Analytics", ": the queries behind every chart"),
					],
				)}
				{band_section(
					"Simulated",
					"pretend",
					&[
						(
							"Email",
							": codes land in your demo inbox and are never sent",
						),
						(
							"Infrastructure",
							": plans and applies run on a stand-in cloud; nothing deploys",
						),
						("Cloud operations", ": no provider is contacted"),
					],
				)}
				{band_section(
					"Shared, and wiped daily",
					"shared",
					&[
						(
							"Everyone using the demo today",
							" sees the same analytics, saved insights, and operations",
						),
						("Don't type anything personal", "."),
						("Everything resets", " at 00:00 UTC"),
					],
				)}
			</View>
		</View>
	}
}

fn band_section(title: &str, kind: &str, items: &[(&str, &str)]) -> Element {
	let dot = match kind {
		"real" => "bg-[#5fd08a]",
		"pretend" => "bg-[#5ef2c2] -rotate-12",
		_ => "bg-[#e3b341]",
	};
	let rows = items
		.iter()
		.map(|(strong, rest)| {
			view! {
				<View class="flex-row items-start gap-2.5 py-0.5">
					<View
						class=format!("mt-[7px] h-1.5 w-1.5 shrink-0 rounded-sm {dot}")
						aria_hidden=true
					/>
					{rich(
						"flex-1 text-[13.5px] leading-normal text-[#c9c0d0]",
						&[(strong, true), (rest, false)],
					)}
				</View>
			}
		})
		.collect::<Vec<_>>();

	view! {
		<View class="flex-1 flex-col gap-1.5" role=Role::Group aria_label=title.to_owned()>
			<Text class="text-xs font-semibold tracking-wider text-[#9d93a8]" aria_hidden=true>
				{title.to_uppercase()}
			</Text>
			{rows}
		</View>
	}
}

/// The app bar every page has: the Notes mark, then whatever the page
/// puts on its right.
pub(crate) fn app_bar(right: Vec<Element>) -> Element {
	view! {
		<View
			class="w-full border-b border-[var(--admin-line)] bg-[var(--admin-surface)]"
			role=Role::Banner
		>
			<View class="w-full max-w-[1120px] self-center h-14 flex-row items-center gap-3 px-4">
				<Pressable
					href="/"
					class="min-h-[44px] flex-row items-center gap-2 text-[15px] font-semibold cursor-pointer"
					aria_label="Notes, the demo's front door"
				>
					<View
						class="h-6 w-6 shrink-0 rounded-md bg-[var(--admin-ember)]"
						aria_hidden=true
					/>
					<Text>"Notes"</Text>
				</Pressable>
				<View class="flex-1" />
				{right}
			</View>
		</View>
	}
}

/// The link that jumps past the chrome, visible only on focus.
pub(crate) fn skip_link() -> Element {
	view! {
		<Pressable
			href="#main"
			test_id="skip"
			aria_label="Skip to the page"
			class="px-3 py-2 text-sm font-semibold"
		>
			<Text>"Skip to the page"</Text>
		</Pressable>
	}
}

/// The polite region that tells a screen reader what just happened without
/// moving focus. It is visually hidden by its test id in
/// `assets/demo.css`, which the class compiler has no utility for.
pub(crate) fn announcer(shell: &Shell) -> Element {
	let announcement = shell.announcement;
	view! {
		<View role=Role::Status aria_live=Live::Polite test_id="announcer">
			<Text>{move || announcement.get()}</Text>
		</View>
	}
}

/// The alert dialog that opens when the demo has reset under an open
/// page. It carries no data of its own: the page opens it when the server
/// names a different day than the one the page was made for.
pub(crate) fn reset_dialog(shell: &Shell) -> Element {
	let reset_open = shell.reset_open;
	view! {
		<Modal
			open=reset_open.get()
			test_id="reset-dialog"
			role=Role::AlertDialog
			aria_labelled_by=["reset-title"]
			aria_described_by=["reset-text"]
			initial_focus="reset-title"
			class="items-center justify-center bg-[#0f0b16]/60 px-4 py-16"
			on:dismiss=move || reset_open.set(false)
		>
			<View class="relative w-full max-w-[440px]">
				<View
					class="absolute inset-0 translate-x-2 translate-y-2 rounded-2xl \
					       bg-[var(--admin-ink)]"
					aria_hidden=true
				/>
				<View class="relative flex-col gap-3.5 rounded-2xl border border-[var(--admin-line)] \
					       bg-[var(--admin-surface)] p-5 md:p-7">
					<View class="flex-row items-center gap-2">
						{piece()} <Text class=LABEL>"00:00 UTC"</Text>
					</View>
					<Heading
						aria_level=2
						id="reset-title"
						test_id="reset-title"
						tab_index=-1
						class="text-2xl font-semibold leading-tight text-[var(--admin-ink)]"
					>
						"The demo just reset."
					</Heading>
					<Text id="reset-text" class=SECONDARY>
						"It's a new day. Yesterday's people and everything saved in the demo were \
						 cleared. This page is from before the reset, so nothing on it works any more."
					</Text>
					{raised(
						view! {
							<Pressable
								href="/"
								test_id="reset-start"
								aria_label="Start again"
								class=format!("{BUTTON_PRIMARY} w-full text-base")
							>
								<Text>"Start again"</Text>
							</Pressable>
						},
						true,
					)}
					<Button
						class=BUTTON_QUIET
						test_id="reset-keep"
						on:press=move || reset_open.set(false)
					>
						"Keep looking at this page"
					</Button>
				</View>
			</View>
		</Modal>
	}
}
