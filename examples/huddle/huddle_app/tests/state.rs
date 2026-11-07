//! [`Huddle`]'s own state machine, without mounting a screen: which channel
//! is open, what is on its way, what the screens say went wrong, and the read
//! marker that follows the newest message. Every call fails the way an
//! offline build's does, so nothing here needs a server.

mod support;

use huddle_api::MessageView;
use huddle_api::People;
use huddle_api::Whoami;
use huddle_app::huddle::Pending;
use huddle_app::huddle::Tab;
use huddle_app::huddle::no_channel;
use leptoskit::leptos::prelude::GetUntracked;
use leptoskit::leptos::prelude::Set;
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use support::Fixture;
use support::finish;
use support::id;
use support::message;
use support::settle;

#[cfg(feature = "csr")]
#[test]
fn the_sidebar_opens_the_first_channel_and_another_when_one_vanishes() {
	let fixture = Fixture::new();
	let huddle = fixture.huddle();

	// The effect follows the sidebar, not the selection: a fresh sidebar
	// answer opens the first card it has when nothing valid is open.
	let cards = support::channel_cards(&fixture.rooms);
	huddle.selected.set(no_channel());
	fixture.set(huddle_api::Channels {}, cards.clone());
	settle();
	assert_eq!(
		huddle.open_channel().map(|card| card.name),
		Some("general".into())
	);

	// The open channel vanishing (its membership went away) opens the next
	// one rather than leaving an empty room on screen.
	huddle.selected.set(id(99));
	fixture.set(huddle_api::Channels {}, cards);
	settle();
	assert_eq!(huddle.selected.get_untracked(), fixture.rooms.general);

	// With nowhere to go it falls back to the nil channel, which no query
	// about a channel matches.
	fixture.set(huddle_api::Channels {}, Vec::new());
	settle();
	assert_eq!(huddle.selected.get_untracked(), no_channel());
	assert!(huddle.open_channel().is_none());

	finish(fixture.owner);
}

#[test]
fn sending_shows_a_pending_copy_until_the_list_has_it() {
	let fixture = Fixture::new();
	let huddle = fixture.huddle();
	let sent = huddle.pending.get_untracked().len();

	// Whitespace is not a message.
	huddle.draft.set("   ".into());
	huddle.send();
	assert_eq!(huddle.pending.get_untracked().len(), sent);
	assert_eq!(huddle.draft.get_untracked(), "   ");

	// A real one is cleared from the draft and shown at once, with the key
	// the server will echo.
	huddle.draft.set("  Friday works too  ".into());
	huddle.send();
	assert_eq!(huddle.draft.get_untracked(), "");
	assert_eq!(huddle.pending.get_untracked().len(), sent + 1);
	let new = huddle
		.pending
		.get_untracked()
		.last()
		.expect("the pending copy")
		.clone();
	assert_eq!(new.body, "Friday works too");
	assert_eq!(new.channel, fixture.rooms.general);
	assert!(!new.failed);

	// The live list holding the same key replaces the pending copy: the
	// screen reads its own write once, not twice.
	let delivered = MessageView {
		id: id(70),
		author_id: fixture.cast.ada.id,
		author: "Ada".into(),
		body: new.body.clone(),
		client_key: new.client_key.clone(),
		sent_at: "2026-11-01T10:00:00Z".into(),
	};
	assert!(
		!huddle
			.pending_here(std::slice::from_ref(&delivered))
			.iter()
			.any(|pending| pending.client_key == new.client_key)
	);
	assert!(
		huddle
			.pending_here(&[])
			.iter()
			.any(|pending| pending.client_key == new.client_key)
	);

	// A call with no server fails, and the row says so.
	settle();
	assert!(
		huddle
			.pending_here(&[])
			.into_iter()
			.find(|pending| pending.client_key == new.client_key)
			.is_some_and(|pending| pending.failed)
	);

	// Retrying an unknown key does nothing; retrying the failed one clears
	// the mark and calls again, then fails again.
	let before = huddle.pending.get_untracked();
	huddle.resend("no-such-key");
	assert_eq!(huddle.pending.get_untracked(), before);
	huddle.resend(&new.client_key);
	assert!(
		huddle
			.pending
			.get_untracked()
			.iter()
			.find(|pending| pending.client_key == new.client_key)
			.is_some_and(|pending| !pending.failed)
	);
	settle();
	assert!(
		huddle
			.pending
			.get_untracked()
			.iter()
			.find(|pending| pending.client_key == new.client_key)
			.is_some_and(|pending| pending.failed)
	);

	finish(fixture.owner);
}

#[test]
fn pending_copies_stay_in_their_own_channel() {
	let fixture = Fixture::new();
	let huddle = fixture.huddle();
	let pending = huddle.pending_here(&[]);

	// The three fixture copies belong to two channels; only the open one's
	// are shown.
	assert!(
		pending
			.iter()
			.all(|copy| copy.channel == fixture.rooms.general)
	);
	assert!(!pending.iter().any(|copy| copy.body == "Another channel"));

	huddle.selected.set(fixture.rooms.launch);
	settle();
	let pending = huddle.pending_here(&[]);
	assert_eq!(pending.len(), 1);
	assert_eq!(pending[0].body, "Another channel");

	finish(fixture.owner);
}

#[test]
fn a_failed_call_says_why_in_the_status_line() {
	let fixture = Fixture::new();
	let huddle = fixture.huddle();

	huddle.problem.set(Some("stale".into()));
	huddle.run(huddle_api::ReadChannel {
		channel: fixture.rooms.general,
	});
	assert_eq!(
		huddle.problem.get_untracked(),
		None,
		"the old problem clears"
	);
	settle();
	let problem = huddle.problem.get_untracked().expect("a problem");
	assert_ne!(problem.len(), 0);

	finish(fixture.owner);
}

#[test]
fn the_channel_and_the_person_come_from_the_cache() {
	let fixture = Fixture::new();
	let huddle = fixture.huddle();

	assert_eq!(
		huddle.open_channel().map(|card| card.name),
		Some("general".into())
	);
	assert_eq!(huddle.my_id(), Some(fixture.cast.ada.id));

	// Signing out, or a cache with no person, leaves no ID to act as.
	fixture.set(Whoami {}, None);
	settle();
	assert_eq!(huddle.my_id(), None);

	// Everyone remains readable for the person pickers.
	settle();
	fixture.set(People {}, Vec::new());
	settle();
	assert!(huddle.people.data().is_some_and(|people| people.is_empty()));

	finish(fixture.owner);
}

#[test]
fn reading_follows_the_newest_message_in_the_open_channel() {
	let fixture = Fixture::new();
	let huddle = fixture.huddle();
	let query = huddle_api::Messages {
		channel: fixture.rooms.general,
		limit: huddle_api::PAGE,
		before: None,
	};

	// No messages, then one, then a second: each change is a reason to mark
	// the channel read. The calls fail here, which the screen doesn't show.
	fixture.set(query.clone(), Vec::new());
	settle();
	fixture.set(
		query.clone(),
		vec![message(5, &fixture.cast.grace, "first", "grace-2")],
	);
	settle();
	fixture.set(
		query.clone(),
		vec![
			message(5, &fixture.cast.grace, "first", "grace-2"),
			message(6, &fixture.cast.lin, "second", "lin-1"),
		],
	);
	settle();
	// Reading the same newest message again doesn't call a second time.
	settle();

	assert_eq!(
		huddle.messages.data().unwrap().last().unwrap().body,
		"second"
	);

	finish(fixture.owner);
}

#[test]
fn defaults_and_fixtures_read_as_expected() {
	assert_eq!(Tab::default(), Tab::Chat);
	assert_eq!(no_channel(), no_channel());
	assert_ne!(no_channel(), id(1));

	let pending = Pending {
		channel: id(1),
		client_key: "key".into(),
		body: "body".into(),
		failed: false,
	};
	assert_eq!(pending.clone(), pending);

	// A mounted empty chat says hello, however the state was reached.
	let fixture = Fixture::new();
	let huddle = fixture.huddle();
	huddle.pending.set(Vec::new());
	fixture.set(
		huddle_api::Messages {
			channel: fixture.rooms.general,
			limit: huddle_api::PAGE,
			before: None,
		},
		Vec::new(),
	);
	settle();
	let screen = fixture.mount(1280.0, 1800.0);
	screen
		.get(&Finder::text("No messages yet. Say hello."))
		.unwrap();
	assert_eq!(huddle.pending.get_untracked().len(), 0);

	drop(screen);
	finish(fixture.owner);
}
