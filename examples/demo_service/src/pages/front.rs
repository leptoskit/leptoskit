//! The front door: who to be, and how sign-in travels to the admin.

use leptoskit::native::Role;
use leptoskit::prelude::*;
use serde_json::json;

use super::Context;
use super::Person;
use super::chrome;
use super::chrome::Tone;
use crate::return_to;

/// What the front door says before the choice.
#[derive(Clone, Debug)]
pub(crate) enum Notice {
	/// Nothing special: a first visit, or a pass that is still good.
	None,
	/// The browser carried a pass from an earlier UTC day. The pass was
	/// cleared; nothing from yesterday exists.
	NewDay,
	/// The visitor just signed out. Their pass, and so their person,
	/// stays theirs until the reset.
	SignedOut,
}

/// What the front door is rendered from.
#[derive(Clone, Debug)]
pub(crate) struct Door {
	pub context: Context,
	/// The validated local path a sign-in should end at.
	pub return_to: String,
	pub notice: Notice,
	/// This browser's own people, none without a pass.
	pub people: Vec<Person>,
}

/// The sign-in link for a person, carrying the validated return target.
fn sign_in_href(persona: &str, return_to: &str) -> String {
	format!(
		"/sign-in?as={persona}&return_to={}",
		return_to::encoded_query_value(return_to)
	)
}

/// The notices above the hero: the new day, welcome back, signed out, and
/// a place for the page's script to say the day is full.
fn notices(door: &Door) -> Vec<Element> {
	let mut notices = Vec::new();

	match door.notice {
		Notice::NewDay => {
			notices.push(chrome::status(
				Tone::Info,
				"notice-new-day",
				"A new demo day started at 00:00 UTC.",
				"Yesterday's people and everything saved in the demo were cleared, including your \
				 Ada or Grace. Pick who to be today.",
				Vec::new(),
			));
		}
		Notice::SignedOut | Notice::None => {}
	}

	let signed_in = door.people.iter().find(|person| person.signed_in);

	if let Some(person) = signed_in {
		let notes = view! {
			<Pressable
				href="/notes"
				test_id="go-notes"
				aria_label="Go to your notes"
				class="self-start text-sm font-semibold  text-[var(--admin-ink-2)] \
				       cursor-pointer"
			>
				<Text>"Go to your notes"</Text>
			</Pressable>
		};
		notices.push(chrome::status(
			Tone::Good,
			"notice-signed-in",
			&format!(
				"You're signed in as {} #{}.",
				person.first_name(),
				person.tag
			),
			"Your person stays yours until 00:00 UTC.",
			vec![notes],
		));
	} else if !door.people.is_empty() {
		let links = door
			.people
			.iter()
			.map(|person| {
				view! {
					<Pressable
						href=sign_in_href(person.persona, &door.return_to)
						test_id=format!("welcome-back:{}", person.persona)
						aria_label=format!("Sign in as {} #{}", person.first_name(), person.tag)
						class="self-start min-h-[28px] text-sm font-semibold  \
						       text-[var(--admin-ink-2)] cursor-pointer"
					>
						<Text>{format!("Sign in as {} #{}", person.first_name(), person.tag)}</Text>
					</Pressable>
				}
			})
			.collect::<Vec<_>>();
		let (title, body) = if matches!(door.notice, Notice::SignedOut) {
			(
				"Signed out of Notes.",
				"Your person stays yours until 00:00 UTC. The admin keeps its own session, so \
				 sign out there too if you opened it.",
			)
		} else {
			(
				"Welcome back.",
				"This browser already has someone to be today. They stay yours until 00:00 UTC.",
			)
		};
		let tone = if matches!(door.notice, Notice::SignedOut) {
			Tone::Good
		} else {
			Tone::Info
		};
		notices.push(chrome::status(
			tone,
			"notice-welcome-back",
			title,
			body,
			links,
		));
	}

	// The day-full and rate-limit answers arrive from the mint, so the
	// script fills this region in rather than the server guessing.
	notices.push(view! {
		<View
			class="flex-col gap-3"
			test_id="mint-notice"
			role=Role::Group
			aria_label="Notices from choosing a person"
		/>
	});

	notices
}

/// One of the two persona cards.
fn persona_card(persona: &'static str) -> Element {
	let ada = persona == "ada";
	let (name, role, blurb, can) = if ada {
		(
			"Ada",
			"admin role",
			"Runs Notes. The admin lets her in to analytics, saved insights, and simulated \
			 infrastructure.",
			[
				("Signs in with a real emailed code", true),
				("Opens the admin: analytics and saved insights", true),
				("Plans a deploy on a stand-in cloud", true),
			],
		)
	} else {
		(
			"Grace",
			"no admin role",
			"Isn't an admin. The admin turns her away, and shows you how that's decided.",
			[
				("Signs in with a real emailed code", true),
				("Gets refused by the admin, on purpose", false),
				("Sees the roles hook that decided it", false),
			],
		)
	};

	let avatar = if ada {
		"bg-[var(--admin-ember)] text-[#fffdf8]"
	} else {
		"bg-[var(--admin-sunken)] border border-[var(--admin-line-strong)] text-[var(--admin-ink)]"
	};
	let chip = if ada {
		"border-[var(--admin-ink)] text-[var(--admin-ink)] font-semibold"
	} else {
		"border-[var(--admin-line-strong)] text-[var(--admin-ink-2)] font-medium"
	};
	let heading_id = format!("persona-{persona}");

	let items = can
		.into_iter()
		.map(|(text, yes)| {
			let dot = if yes {
				"bg-[var(--admin-ember)]"
			} else {
				"bg-[var(--admin-line-strong)]"
			};
			view! {
				<View class="flex-row items-start gap-2 py-0.5" role=Role::ListItem>
					<View
						class=format!("mt-[7px] h-[7px] w-[7px] shrink-0 rounded-sm {dot}")
						aria_hidden=true
					/>
					<Text class="flex-1 text-[13.5px] leading-normal text-[var(--admin-ink-2)]">
						{text}
					</Text>
				</View>
			}
		})
		.collect::<Vec<_>>();

	let button = if ada {
		chrome::raised(
			view! {
				<Button class=format!("{} w-full", chrome::BUTTON_PRIMARY) test_id="mint:ada">
					"Continue as Ada"
				</Button>
			},
			false,
		)
	} else {
		view! {
			<Button class=format!("{} self-start", chrome::BUTTON_SECONDARY) test_id="mint:grace">
				"Continue as Grace"
			</Button>
		}
	};

	view! {
		<View
			class="flex-col gap-3.5 rounded-xl border border-[var(--admin-line)] \
			       bg-[var(--admin-surface)] p-5"
			role=Role::Article
			aria_labelled_by=[heading_id.clone()]
		>
			<View class="flex-row items-center gap-3">
				<View
					class=format!(
						"h-11 w-11 shrink-0 items-center justify-center rounded-xl {avatar}"
					)
					aria_hidden=true
				>
					<Text class="text-[19px] font-bold">
						{name.chars().next().map(String::from).unwrap_or_default()}
					</Text>
				</View>
				<View class="flex-col items-start gap-1.5">
					<Heading
						aria_level=2
						id=heading_id
						class="text-xl font-bold leading-none text-[var(--admin-ink)]"
					>
						{name}
					</Heading>
					<View class=format!(
						"rounded-full border px-2.5 py-px bg-[var(--admin-surface)] {chip}"
					)>
						<Text class="text-xs">{role}</Text>
					</View>
				</View>
			</View>
			<Text class=chrome::SECONDARY>{blurb}</Text>
			<View class="flex-col gap-1" role=Role::List aria_label=format!("What {name} can do")>
				{items}
			</View>
			<View class="mt-auto pt-1" test_id=format!("persona-actions:{persona}")>{button}</View>
		</View>
	}
}

/// "Notes → signs you in to → Notes admin", with the Workings drawer that
/// explains the ID token and its `roles` claim.
fn route(door: &Door) -> Element {
	let node = |host: &str, name: &str, what: &str| {
		view! {
			<View class="flex-1 min-w-0 flex-col gap-0.5 rounded-xl border border-[var(--admin-line)] \
				       bg-[var(--admin-surface)] px-4 py-3.5">
				<Text class="font-[var(--admin-font-mono)] text-xs text-[var(--admin-ink-3)] ">
					{host.to_owned()}
				</Text>
				<Text class="text-[15px] font-semibold text-[var(--admin-ink)]">
					{name.to_owned()}
				</Text>
				<Text class="text-[13px] text-[var(--admin-ink-2)]">{what.to_owned()}</Text>
			</View>
		}
	};

	view! {
		<View class="flex-col gap-3" role=Role::Region aria_labelled_by=["route-title"]>
			<View class="flex-row flex-wrap items-center gap-3">
				<Heading
					aria_level=2
					id="route-title"
					class="text-[13px] font-semibold text-[var(--admin-ink-2)]"
				>
					"How sign-in travels"
				</Heading>
				<Button
					test_id="route-toggle"
					aria_expanded=false
					aria_controls=["route-workings"]
					class="flex-row items-center gap-1.5 rounded-full border border-[var(--admin-mint)]/50 \
					       bg-[var(--admin-mint-wash)] py-0.5 pl-1.5 pr-2.5 text-xs font-semibold \
					       text-[var(--admin-mint-text)] cursor-pointer min-h-[28px]"
				>
					<View
						class="h-3 w-3 shrink-0 rounded-[3px] bg-[var(--admin-mint)] -rotate-12"
						aria_hidden=true
					/>
					<Text>"Workings"</Text>
				</Button>
			</View>
			<View class="flex-col gap-3 md:flex-row md:items-stretch" test_id="route">
				{node(
					&door.context.site_host,
					"Notes",
					"Accounts, sign-in codes, roles",
				)}
				<View
					class="shrink-0 flex-col items-center justify-center gap-1 px-2 py-1 text-center \
					       md:w-[150px]"
					aria_label="signs you in to, with OpenID Connect"
				>
					<Text
						class="text-xs font-semibold text-[var(--admin-mint-text)]"
						aria_hidden=true
					>
						"signs you in to"
					</Text>
					<Text
						class="text-lg font-semibold leading-none text-[var(--admin-mint)]"
						aria_hidden=true
						test_id="route-arrow"
					>
						"→"
					</Text>
					<Text
						class="font-[var(--admin-font-mono)] text-[11.5px] text-[var(--admin-ink-3)]"
						aria_hidden=true
					>
						"OpenID Connect"
					</Text>
				</View>
				{node(
					&door.context.admin_host,
					"Notes admin",
					"Analytics, saved insights, simulated infrastructure",
				)}
			</View>
			<View
				id="route-workings"
				test_id="route-workings"
				class="flex-col gap-2 rounded-xl border border-[var(--admin-underside-line)] \
				       bg-[var(--admin-underside)] p-4"
			>
				{chrome::rich(
					"text-[13.5px] leading-relaxed text-[#c9c0d0]",
					&[
						(
							"The admin has no accounts of its own. It sends you to Notes; Notes \
							 signs you in with a code and hands the admin an ID token saying who \
							 you are and which ",
							false,
						),
						("roles", true),
						(" you have. Ada's token says ", false),
						("[\"admin\"]", true),
						(
							". Grace's says nothing, so the admin refuses her. Two hostnames, so \
							 the two sessions never share a cookie.",
							false,
						),
					],
				)}
			</View>
		</View>
	}
}

fn body(door: &Door) -> Element {
	view! {
		<View
			id="main"
			role=Role::Main
			tab_index=-1
			class=format!("{} flex-col gap-8 py-8 md:py-10", chrome::COLUMN)
		>
			{notices(door)}
			<View class="flex-col items-start gap-4">
				<View class="flex-row items-center gap-2 rounded-full border border-[var(--admin-line-strong)] \
					       bg-[var(--admin-surface)] py-0.5 pl-2 pr-3">
					{chrome::mark(16.0)}
					<Text class="text-[12.5px] font-semibold text-[var(--admin-ink-2)]">
						"A leptoskit demo"
					</Text>
				</View>
				<Heading
					aria_level=1
					id="front-title"
					tab_index=-1
					class="max-w-[620px] text-3xl font-bold leading-[1.1] tracking-tight md:text-5xl \
					       text-[var(--admin-ink)]"
				>
					"Be Ada or Grace for a day."
				</Heading>
				<Text class="max-w-[560px] text-base leading-relaxed text-[var(--admin-ink-2)] md:text-[17px]">
					"Notes is a small note-taking app built with leptoskit, with a real admin beside \
					 it. Pick a person, sign in with a real emailed code, and see how the admin \
					 decides who gets in."
				</Text>
			</View>
			<View
				class="grid grid-cols-1 gap-4 md:grid-cols-2"
				role=Role::Group
				aria_label="Choose who to be"
			>
				{persona_card("ada")} {persona_card("grace")}
			</View>
			<Text class=format!("max-w-[700px] {}", chrome::MUTED)>
				"You get your own Ada or Grace: a made-up account at a made-up address that only \
				 this browser can sign in to. Nobody else shares it, and it's gone at 00:00 UTC."
			</Text>
			{route(door)}
		</View>
	}
}

/// The front door as a document.
pub(super) fn page(door: &Door) -> String {
	let right = vec![view! {
		<Pressable
			href=format!(
				"/sign-in?return_to={}",
				return_to::encoded_query_value(&door.return_to)
			)
			test_id="app-bar-sign-in"
			aria_label="Sign in"
			class="min-h-[44px] flex-row items-center px-2 text-sm font-semibold \
			       text-[var(--admin-ink-2)] cursor-pointer"
		>
			<Text>"Sign in"</Text>
		</Pressable>
	}];

	let tree = view! {
		<View class=chrome::PAGE>
			{chrome::skip_link()} {chrome::band(&door.context)} {chrome::app_bar(right)}
			{body(door)} {chrome::reset_dialog()}
		</View>
	};

	let mut state = super::base_state("front", &door.context, &door.return_to);
	state["notice"] = json!(match door.notice {
		Notice::None => "none",
		Notice::NewDay => "new-day",
		Notice::SignedOut => "signed-out",
	});
	state["people"] = json!(door.people);

	super::document(
		"Notes: a leptoskit demo",
		" data-page=\"front\"",
		&state,
		tree,
		&noscript_form(&door.return_to),
	)
}

/// What a visitor without scripts gets: the same choice as a plain form
/// post to the mint, so the front door never depends on JavaScript.
fn noscript_form(return_to: &str) -> String {
	format!(
		"<noscript><form class=\"plain-mint\" method=\"post\" action=\"/demo/pass\"><p>This page \
		 runs better with scripts on. Without them you can still choose who to be:</p><input \
		 type=\"hidden\" name=\"return_to\" value=\"{}\"><button name=\"persona\" \
		 value=\"ada\">Continue as Ada</button> <button name=\"persona\" value=\"grace\">Continue \
		 as Grace</button></form></noscript>",
		super::escape(return_to)
	)
}
