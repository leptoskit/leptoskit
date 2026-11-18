//! `TurnCard`: every card has a back, the Workings, that says where its
//! numbers came from (`docs/agents/cloud-design.md` §1 and §5).
//!
//! The mint tab turns one card; `W` turns the focused one, `Shift+W` turns
//! every card, staggered 40ms apart, and `Esc` turns them all back. The face
//! that arrives opens from its middle outwards, a 150ms clip that reads as
//! the card turning; people who ask for reduced motion see the faces swap at
//! once, and so do native apps, where `Motion` shows elements at rest.

use std::collections::BTreeMap;
use std::time::Duration;

use leptoskit::prelude::*;

use super::Kind;
use crate::style;

/// How long the arriving face takes to open.
pub const TURN: Duration = Duration::from_millis(150);

/// How far apart cards turn when they all turn at once.
pub const TURN_ALL_STAGGER: Duration = Duration::from_millis(40);

/// Which cards are turned over. It's `Copy`, so handlers capture it freely.
#[derive(Clone, Copy, Debug)]
pub struct Turned {
	/// Whether `Shift+W` turned every card, and how many times every card has
	/// turned together, which replays the motion.
	all: RwSignal<(bool, u64)>,
	/// Cards turned one at a time since: whether each shows its back, and
	/// how many times it has turned.
	cards: RwSignal<BTreeMap<String, (bool, u64)>>,
	/// The card whose controls have focus, which `W` turns.
	focused: RwSignal<Option<String>>,
}

impl Default for Turned {
	fn default() -> Self {
		Self::new()
	}
}

impl Turned {
	/// Every card on its front.
	#[must_use]
	pub fn new() -> Self {
		Self {
			all: RwSignal::new((false, 0)),
			cards: RwSignal::new(BTreeMap::new()),
			focused: RwSignal::new(None),
		}
	}

	/// Whether card `id` shows its back.
	#[must_use]
	pub fn is_turned(&self, id: &str) -> bool {
		let all = self.all.get().0;
		self.cards
			.with(|cards| cards.get(id).map_or(all, |(back, _)| *back))
	}

	/// How many times card `id` has turned, which keys its motion. `0` until
	/// it first turns, so nothing moves when the page loads.
	#[must_use]
	pub fn turns(&self, id: &str) -> u64 {
		let together = self.all.get().1;
		together
			+ self
				.cards
				.with(|cards| cards.get(id).map_or(0, |(_, turns)| *turns))
	}

	/// Whether card `id` last turned on its own, rather than with every card.
	fn turned_alone(&self, id: &str) -> bool {
		self.cards.with(|cards| cards.contains_key(id))
	}

	/// Turns card `id` over, or back.
	pub fn turn(&self, id: &str) {
		let back = !self.is_turned_untracked(id);
		self.cards.update(|cards| {
			let turns = cards.get(id).map_or(0, |(_, turns)| *turns);
			cards.insert(id.to_owned(), (back, turns + 1));
		});
		self.focused.set(Some(id.to_owned()));
	}

	fn is_turned_untracked(&self, id: &str) -> bool {
		let all = self.all.get_untracked().0;
		self.cards
			.with_untracked(|cards| cards.get(id).map_or(all, |(back, _)| *back))
	}

	/// Turns the focused card, as `W` does. Says whether a card had focus.
	pub fn turn_focused(&self) -> bool {
		match self.focused.get_untracked() {
			Some(id) => {
				self.turn(&id);
				true
			}
			None => false,
		}
	}

	/// Turns every card over, as `Shift+W` does.
	pub fn turn_all(&self) {
		self.all.update(|(back, turns)| {
			*back = true;
			*turns += 1;
		});
		self.cards.set(BTreeMap::new());
	}

	/// Turns every card back, as `Esc` does. Says whether any was turned.
	pub fn turn_back_all(&self) -> bool {
		let any = self.all.get_untracked().0
			|| self
				.cards
				.with_untracked(|cards| cards.values().any(|(back, _)| *back));

		if any {
			self.all.update(|(back, turns)| {
				*back = false;
				*turns += 1;
			});
			self.cards.set(BTreeMap::new());
		}

		any
	}

	/// Notes that a control on card `id` has focus, or that none does.
	pub fn focus(&self, id: Option<&str>) {
		self.focused.set(id.map(str::to_owned));
	}

	fn is_focused(&self, id: &str) -> bool {
		self.focused
			.with_untracked(|focused| focused.as_deref() == Some(id))
	}
}

/// The mint tab: the logo's mint piece and `Workings`, naming the card's
/// source for assistive technology.
pub fn mint_tab(drawer: &str, source: &str) -> Element {
	Pressable(format!("Show workings: {source}"))
		.class(style::MINT_TAB)
		.test_id(format!("workings-tab:{drawer}"))
		.aria_expanded(false)
		.child(
			View()
				.class(style::MINT_PIECE)
				.aria_hidden(true)
				.test_id("mint-piece"),
		)
		.child(Text("Workings"))
}

/// A card with a front and a back.
pub struct TurnCard {
	/// The card's ID: unique on the screen.
	pub id: String,
	/// The card's title, on both faces.
	pub title: String,
	/// Where the card's data comes from, such as `projects::projects`.
	pub source: String,
	/// Its place on the screen, which staggers turning every card.
	pub index: usize,
	/// The front's body.
	pub front: Vec<Element>,
	/// The back's body: how the front's facts came about.
	pub back: Vec<Element>,
}

impl TurnCard {
	/// The card, on whichever face `turned` says.
	#[must_use]
	pub fn build(self, turned: Turned) -> Element {
		let id = self.id.clone();
		let back = turned.is_turned(&id);
		let turns = turned.turns(&id);
		// Focus follows the card that was turned on its own, onto the control
		// on the face that arrived.
		let take_focus = turns > 0 && turned.turned_alone(&id) && turned.is_focused(&id);
		let face = if back {
			self.back_face(turned, take_focus)
		} else {
			self.front_face(turned, take_focus)
		};
		let face = if turns == 0 {
			face
		} else {
			let delay = if turned.turned_alone(&id) {
				Duration::ZERO
			} else {
				TURN_ALL_STAGGER.saturating_mul(u32::try_from(self.index).unwrap_or(u32::MAX))
			};
			face.motion(
				Motion::enter(Pose::new().clip(Inset::new(0.0, 0.5, 0.0, 0.5)))
					.duration(TURN)
					.delay(delay)
					.easing(Easing::Out)
					.key(turns),
			)
		};

		View()
			.class("relative flex-col")
			.test_id(format!("card:{id}"))
			.child(face)
	}

	fn front_face(&self, turned: Turned, take_focus: bool) -> Element {
		let id = self.id.clone();
		let focus_id = self.id.clone();
		// Each card's tab names its card, so a screen of cards from one source
		// doesn't read as a row of identical buttons.
		let tab = mint_tab(&self.id, &self.source)
			.accessibility_label(format!("Show workings for {}: {}", self.title, self.source))
			.focused(take_focus)
			.on_press(move || turned.turn(&id))
			.on(NativeEventSlot::Focus, move |_| {
				turned.focus(Some(&focus_id));
				EventOutcome::Unhandled
			});

		View()
			.class(style::CARD)
			.child(
				View()
					.class(style::CARD_HEADER)
					.child(Heading(2, self.title.clone()).class(style::CARD_TITLE))
					.child(View().class("grow"))
					.child(tab),
			)
			.children(self.front.iter().cloned())
	}

	fn back_face(&self, turned: Turned, take_focus: bool) -> Element {
		let id = self.id.clone();
		let focus_id = self.id.clone();
		let back = Pressable(format!("Turn back: {}", self.title))
			.class(Kind::Quiet.class_name())
			.class("text-[#f4efe8] hover:bg-[var(--cloud-underside-line)]")
			.aria_expanded(true)
			.focused(take_focus)
			.test_id(format!("turn-back:{}", self.id))
			.on_press(move || turned.turn(&id))
			.on(NativeEventSlot::Focus, move |_| {
				turned.focus(Some(&focus_id));
				EventOutcome::Unhandled
			})
			.child(Text("Turn back"));

		View()
			.class(style::UNDERSIDE)
			.child(
				View()
					.class(style::CARD_HEADER)
					.child(View().class(style::MINT_PIECE).aria_hidden(true))
					.child(
						Heading(2, format!("Workings: {}", self.title))
							.class("text-base font-semibold text-[#f4efe8]"),
					)
					.child(View().class("grow"))
					.child(back),
			)
			.child(
				Text(format!("From {}", self.source))
					.class("font-[var(--cloud-font-mono)] text-[13px] text-[var(--cloud-mint)]"),
			)
			.children(self.back.iter().cloned())
	}
}
