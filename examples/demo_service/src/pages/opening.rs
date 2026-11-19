//! The page the service answers with while today's generation builds.
//!
//! It is plain, safe without scripts, and says only what is known: the
//! demo is not ready yet. It never claims a reset happened, because a
//! release, a failed start, and midnight all look the same from here.

use leptoskit::native::Role;
use leptoskit::prelude::*;

/// Body text, and the quieter line under it.
const SECONDARY: &str = "text-sm leading-normal text-[var(--admin-ink-2)]";
const MUTED: &str = "text-[13px] leading-normal text-[var(--admin-ink-3)]";

/// The seconds between the page's checks. The response's `Retry-After`
/// and its meta refresh both say this.
pub(crate) const RETRY_SECONDS: u32 = 5;

/// "Notes is opening today's demo", as a document: its stylesheet is
/// inline and it asks for nothing else. Rendered once, because the page
/// holds nothing about the visitor or the day.
pub(crate) fn opening() -> &'static str {
	static PAGE: std::sync::LazyLock<String> = std::sync::LazyLock::new(render);
	&PAGE
}

fn render() -> String {
	let tree = view! {
		<View class="min-h-screen w-full items-center justify-center bg-[var(--admin-paper)] px-4 py-12 \
			       text-[var(--admin-ink)]">
			<View
				id="main"
				role=Role::Main
				class="w-full max-w-[420px] flex-col items-center gap-4 text-center"
			>
				<View class="relative h-[84px] w-[84px]" aria_hidden=true test_id="assemble">
					<View
						class="absolute left-0 top-3 h-[72px] w-[33px] rounded-[9px] bg-[var(--admin-ember)]"
						test_id="assemble-1"
					/>
					<View
						class="absolute left-[42px] top-[51px] h-[33px] w-[33px] rounded-[9px] bg-[var(--admin-ember-warm)]"
						test_id="assemble-2"
					/>
					<View
						class="absolute left-[45px] top-0 h-[33px] w-[33px] rounded-[9px] bg-[var(--admin-mint)] -rotate-12"
						test_id="assemble-3"
					/>
				</View>
				<Heading
					aria_level=1
					class="text-2xl font-semibold leading-tight text-[var(--admin-ink)]"
				>
					"Notes is opening today's demo."
				</Heading>
				<View role=Role::Status class="flex-col gap-3">
					<Text class=SECONDARY>
						{format!(
							"The demo starts fresh every day at 00:00 UTC, and after each \
							 release. This takes a few seconds; the page checks again every \
							 {RETRY_SECONDS} seconds."
						)}
					</Text>
					<Text class=MUTED>
						"If this lasts more than a minute, the demo is down for maintenance. Try again \
						 later."
					</Text>
				</View>
			</View>
		</View>
	};

	let body = leptos::prelude::RenderHtml::to_html(leptoskit::web::render(tree));
	format!(
		"<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" \
		 content=\"width=device-width, initial-scale=1, viewport-fit=cover\"><meta \
		 name=\"color-scheme\" content=\"light dark\"><meta name=\"robots\" \
		 content=\"noindex\"><meta http-equiv=\"refresh\" \
		 content=\"{RETRY_SECONDS}\"><title>Notes is opening today's \
		 demo</title><style>{theme}{STYLE}</style></head><body>{body}</body></html>",
		theme = leptoskit_admin::theme_css(),
		STYLE = OPENING_STYLE,
	)
}

/// The page's own rules: the body's paper, and the mark assembling,
/// which reduced motion turns into the finished mark.
const OPENING_STYLE: &str =
	"html,body{margin:0;background:var(--admin-paper)}body{font-family:var(--admin-font-sans);\
	 -webkit-font-smoothing:antialiased}[data-testid=assemble-1]{animation:drop1 2.4s ease-out \
	 infinite}[data-testid=assemble-2]{animation:drop2 2.4s ease-out \
	 infinite}[data-testid=assemble-3]{animation:land 2.4s ease-out infinite}@keyframes \
	 drop1{0%{opacity:0;transform:translateY(-10px)}15%,85%{opacity:1;transform:none}100%{opacity:\
	 0}}@keyframes \
	 drop2{0%,20%{opacity:0;transform:translateY(-10px)}35%,85%{opacity:1;transform:none}100%\
	 {opacity:0}}@keyframes land{0%,40%{opacity:0;transform:translateY(-16px) \
	 rotate(20deg)}60%,85%{opacity:1;transform:rotate(-12deg)}100%{opacity:0;transform:\
	 rotate(-12deg)}}@media \
	 (prefers-reduced-motion:reduce){*{animation:none!important;transition:none!important}}";
