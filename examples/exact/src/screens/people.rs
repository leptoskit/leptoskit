//! Spark, Expose, messages, Exact Live, sparkline, and Conduit.

use leptoskit::prelude::*;

use crate::frame::Card;
use crate::frame::Choice;
use crate::frame::Frame;
use crate::frame::Note;

const CARDS: [&str; 3] = ["Coast", "Archive", "Night market"];

/// A swipe deck. Throw advances the card.
#[component]
pub fn Spark() -> Element {
	let top = remember(|| 0_usize);
	let index = top.get() % CARDS.len();
	let title = CARDS[index];
	view! {
		<Frame title="Spark">
			<Note text="Throw a card. It leaves, and the next one is already there." />
			<Card>
				<Text class="text-base font-semibold text-slate-900">{title}</Text>
				<Choice label="Throw" on_press=move || top.update(|value| *value += 1) />
			</Card>
		</Frame>
	}
}

/// A small phone: lock screen, then home.
#[component]
pub fn Expose() -> Element {
	let unlocked = remember(|| false);
	let open = unlocked.get();
	view! {
		<Frame title="Expose">
			<Note text="Lock screen, then home: Messages and Reader." />
			{if open {
				view! {
					<Card>
						<Text class="text-sm font-semibold text-slate-900">"Messages"</Text>
						<Text class="text-sm text-slate-600">"Reader"</Text>
					</Card>
				}
			} else {
				view! {
					<Card>
						<Text class="text-2xl font-bold text-slate-900">"9:41"</Text>
						<Choice label="Unlock" on_press=move || unlocked.set(true) />
					</Card>
				}
			}}
		</Frame>
	}
}

/// A short chat thread and one scripted reply.
#[component]
pub fn Messages() -> Element {
	let sent = remember(|| false);
	let replied = sent.get();
	view! {
		<Frame title="Messages">
			<Note text="A chat. The reply is scripted, so it runs offline." />
			<Card>
				<Text class="text-sm text-slate-900">"Are you on the 5:40?"</Text>
				{replied.then(|| {
					view! { <Text class="text-sm text-indigo-600">"Yes — last car."</Text> }
				})}
				<Choice label="Reply" on_press=move || sent.set(true) />
			</Card>
		</Frame>
	}
}

const LIVE: [&str; 3] = ["Photo grid", "Crew chat", "Runbook"];

/// Photos, a crew chat, and a runbook, one at a time.
#[component]
pub fn ExactLive() -> Element {
	let tab = remember(|| 0_usize);
	let index = tab.get();
	view! {
		<Frame title="Exact Live">
			<Note text="Reorder photos, talk to the crew, read the runbook." />
			<View class="flex-row flex-wrap gap-2">
				<Choice label="Photos" on_press=move || tab.set(0) />
				<Choice label="Crew" on_press=move || tab.set(1) />
				<Choice label="Runbook" on_press=move || tab.set(2) />
			</View>
			<Card>
				<Text class="text-sm font-semibold text-slate-900">{LIVE[index]}</Text>
			</Card>
		</Frame>
	}
}

/// A short market list. The benchmark draws the long one.
#[component]
pub fn Sparkline() -> Element {
	view! {
		<Frame title="Sparkline">
			<Note text="A market list. Each row is a name, a price, and a chart." />
			<Coin name="Ada" price="0.42" />
			<Coin name="Sol" price="148" />
			<Coin name="Dot" price="4.10" />
		</Frame>
	}
}

#[component]
fn Coin(name: &'static str, price: &'static str) -> Element {
	view! {
		<Card>
			<View class="flex-row items-center gap-3">
				<Text class="flex-1 text-sm font-semibold text-slate-900">{name}</Text>
				<Text class="text-sm text-slate-700">{price}</Text>
				<View class="flex-row items-end gap-px h-6">
					<View class="w-1 h-2 bg-indigo-500" />
					<View class="w-1 h-4 bg-indigo-500" />
					<View class="w-1 h-3 bg-indigo-500" />
					<View class="w-1 h-5 bg-indigo-500" />
				</View>
			</View>
		</Card>
	}
}

/// A small magazine: a feed, then one article.
#[component]
pub fn Conduit() -> Element {
	let article = remember(|| false);
	let open = article.get();
	view! {
		<Frame title="Conduit">
			<Note text="The RealWorld feed, against sample posts." />
			{if open {
				view! {
					<Card>
						<Text class="text-base font-semibold text-slate-900">
							"How the board stays fast"
						</Text>
						<Text class="text-sm text-slate-600">"Draw the rows you can see."</Text>
					</Card>
				}
			} else {
				view! {
					<Card>
						<Text class="text-sm font-semibold text-slate-900">"Global feed"</Text>
						<Choice label="Open article" on_press=move || article.set(true) />
					</Card>
				}
			}}
		</Frame>
	}
}
