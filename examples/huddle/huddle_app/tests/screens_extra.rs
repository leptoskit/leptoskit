//! The signed-in screens in their less common states: every board column and
//! move, every invitation and tip status, a private channel with a pinned
//! summary and a problem, a channel being created, an empty channel, and the
//! bell with noticed actions. Each action runs for real, and fails the way an
//! offline build's does, so every handler is exercised without a server.

mod support;

use huddle_api::RetryTip;
use huddle_app::huddle::Tab;
use huddle_app::huddle::no_channel;
use leptoskit::leptos::prelude::Set;
use leptoskit_rpc::Inbox;
use leptoskit_rpc::InboxArgs;
use leptoskit_rpc::InboxNotice;
use leptoskit_rpc::Notice;
use leptoskit_rpc::NoticeAction;
use leptoskit_rpc::NoticeKind;
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use support::Fixture;
use support::channel_card;
use support::finish;
use support::id;
use support::settle;

#[test]
fn the_board_moves_cards_in_every_direction() {
	let fixture = Fixture::new();
	let huddle = fixture.huddle();
	huddle.tab.set(Tab::Board);
	settle();
	let mut screen = fixture.mount(1280.0, 2600.0);

	screen.get(&Finder::text("To do · 2")).unwrap();
	screen.get(&Finder::text("Doing · 1")).unwrap();
	screen.get(&Finder::text("Done · 1")).unwrap();

	// Two cards in `todo`: the first can go down, the second up.
	let down = Finder::label("Move Write the launch post down");
	let up = Finder::label("Move Review the copy up");
	screen.get(&down).unwrap();
	screen.get(&up).unwrap();
	// Every column but the last offers a step right, and every column but
	// the first a step left.
	screen
		.get(&Finder::label("Move Write the launch post to Doing"))
		.unwrap();
	screen
		.get(&Finder::label("Move Fix sign-in to To do"))
		.unwrap();
	screen
		.get(&Finder::label("Move Fix sign-in to Done"))
		.unwrap();
	screen.get(&Finder::label("Move Ship it to Doing")).unwrap();

	assert!(
		screen
			.query_by_label("Move Ship it down")
			.unwrap()
			.is_none(),
		"the last card in a column can't move down"
	);

	screen.tap(&down).unwrap();
	settle();
	screen.pump();
	screen.tap(&up).unwrap();
	settle();
	screen.pump();

	// A new card needs a title, then runs the mutation it names.
	screen
		.enter_text(&Finder::label("New card title"), "Draft the changelog")
		.unwrap();
	screen.tap(&Finder::text("Add card")).unwrap();
	settle();
	screen.pump();
	assert!(
		screen
			.query_by_text("Draft the changelog")
			.unwrap()
			.is_none(),
		"the card waits for the server's row"
	);

	drop(screen);
	finish(fixture.owner);
}

#[test]
fn the_activity_panel_shows_every_invitation_tip_and_membership_state() {
	let fixture = Fixture::new();
	let huddle = fixture.huddle();
	huddle.tab.set(Tab::Activity);
	settle();
	let mut screen = fixture.mount(1280.0, 3800.0);

	// Invitations: each status gets its own words.
	screen.get(&Finder::text("lin@huddle.dev")).unwrap();
	screen.get(&Finder::text("Email sent")).unwrap();
	screen.get(&Finder::text("Sending…")).unwrap();
	screen.get(&Finder::text("Couldn't send")).unwrap();

	// Tips: sent and received, and every settlement state.
	screen
		.get(&Finder::text("5 coins to Grace Hopper"))
		.unwrap();
	screen
		.get(&Finder::text(
			"Failed: The settlement service declined the tip (simulated)",
		))
		.unwrap();
	screen.get(&Finder::text("7 coins to Lin")).unwrap();
	screen.get(&Finder::text("Confirmed")).unwrap();
	screen
		.get(&Finder::text("3 coins from Grace Hopper"))
		.unwrap();
	screen.get(&Finder::text("Pending")).unwrap();
	screen.get(&Finder::text("4 coins from Lin")).unwrap();
	screen
		.get(&Finder::text("Submitted, waiting for confirmation"))
		.unwrap();

	// Only the sender of a failed tip may retry it.
	screen
		.get(&Finder::label("Retry the tip to Grace Hopper"))
		.unwrap();
	assert!(
		screen
			.query_by_label("Retry the tip to Lin")
			.unwrap()
			.is_none(),
		"a confirmed tip has nothing to retry"
	);
	screen
		.tap(&Finder::label("Retry the tip to Grace Hopper"))
		.unwrap();
	settle();
	screen.pump();

	// Coins are typed as text and parsed; a number in range enables tipping.
	screen
		.enter_text(&Finder::label("Coins to tip"), "250")
		.unwrap();
	screen
		.get(&Finder::label("Tip Grace Hopper 250 coins"))
		.unwrap();
	screen
		.tap(&Finder::label("Tip Grace Hopper 250 coins"))
		.unwrap();
	settle();
	screen.pump();
	// Something outside the range disables every tip button.
	screen
		.enter_text(&Finder::label("Coins to tip"), "5000")
		.unwrap();
	assert_eq!(
		screen
			.get(&Finder::label("Tip Grace Hopper 5000 coins"))
			.unwrap()
			.props
			.enabled,
		Some(false),
		"5000 coins is outside what a tip may carry"
	);

	// Reminders run a mutation and report it pending.
	screen
		.get(&Finder::text("A notice about this channel, in 10 minutes."))
		.unwrap();
	screen
		.tap(&Finder::text("Remind me in 10 minutes"))
		.unwrap();
	settle();
	screen.pump();

	// An invite needs an address with an `@` in it.
	screen
		.enter_text(&Finder::label("Email address to invite"), "pat@huddle.dev")
		.unwrap();
	screen.tap(&Finder::text("Invite")).unwrap();
	settle();
	screen.pump();

	// The channel is Ada's, so she decides who is in it. Grace is a member,
	// Lin isn't, and Ada can't remove herself.
	screen
		.get(&Finder::text("Lin (not in this channel)"))
		.unwrap();
	screen
		.get(&Finder::label("Remove Grace Hopper from this channel"))
		.unwrap();
	screen
		.get(&Finder::label("Add Lin to this channel"))
		.unwrap();
	screen
		.get(&Finder::label("Sign Grace Hopper out everywhere"))
		.unwrap();
	assert!(
		screen
			.query_by_label("Remove Ada from this channel")
			.unwrap()
			.is_none(),
		"nobody removes themselves"
	);

	screen
		.tap(&Finder::label("Remove Grace Hopper from this channel"))
		.unwrap();
	settle();
	screen.pump();
	screen
		.tap(&Finder::label("Add Lin to this channel"))
		.unwrap();
	settle();
	screen.pump();
	screen
		.tap(&Finder::label("Sign Grace Hopper out everywhere"))
		.unwrap();
	settle();

	screen.pump();

	drop(screen);
	finish(fixture.owner);
}

#[test]
fn the_sidebar_switches_channels_and_creates_new_ones() {
	let fixture = Fixture::new();
	let mut screen = fixture.mount(1280.0, 1800.0);

	// Three unread says so in the row and in the spoken label.
	screen.get(&Finder::label("general, channel")).unwrap();
	screen.get(&Finder::text("#launch (3)")).unwrap();
	screen
		.get(&Finder::label("launch, channel, 3 unread"))
		.unwrap();
	screen.get(&Finder::label("general, channel")).unwrap();

	// Switching channels shows that channel's own state: the pending copy
	// addresses the other channel, so it disappears from view.
	screen.tap(&Finder::text("#launch (3)")).unwrap();
	settle();
	screen.pump();
	screen.get(&Finder::text("#launch")).unwrap();
	screen.get(&Finder::text("Private")).unwrap();
	screen.get(&Finder::text("Another channel")).unwrap();
	assert!(
		screen
			.query_by_text("Ship it on Thursday?")
			.unwrap()
			.is_none(),
		"the other channel's history is gone"
	);

	// One unread is singular.
	fixture.set(
		huddle_api::Channels {},
		vec![
			channel_card(fixture.rooms.general, "general", false, true, 0),
			channel_card(fixture.rooms.launch, "launch", true, false, 1),
		],
	);
	settle();
	screen.pump();
	screen
		.get(&Finder::label("launch, channel, 1 unread"))
		.unwrap();
	screen.get(&Finder::text("#launch (1)")).unwrap();

	// Back to `general`, then start a channel both ways.
	screen.tap(&Finder::label("general, channel")).unwrap();
	settle();
	screen.pump();
	screen
		.enter_text(&Finder::label("New channel name"), "design")
		.unwrap();
	screen.tap(&Finder::text("Create")).unwrap();
	settle();
	screen.pump();
	screen
		.enter_text(&Finder::label("New channel name"), "secret")
		.unwrap();

	screen
		.tap(&Finder::label("Create a private channel"))
		.unwrap();
	settle();
	screen.pump();
	// Both presses clear the field, so neither waits a second time.
	assert!(
		screen.query_by_text("Create").unwrap().is_some(),
		"the row stays"
	);

	drop(screen);
	finish(fixture.owner);
}

#[test]
fn an_empty_channel_invites_a_hello() {
	let fixture = Fixture::new();
	let huddle = fixture.huddle();
	huddle.pending.set(Vec::new());
	fixture.set(
		huddle_api::Messages {
			channel: fixture.rooms.launch,
			limit: huddle_api::PAGE,
			before: None,
		},
		Vec::new(),
	);
	huddle.selected.set(fixture.rooms.launch);
	settle();
	let screen = fixture.mount(1280.0, 1800.0);

	screen
		.get(&Finder::text("No messages yet. Say hello."))
		.unwrap();
	screen.get(&Finder::label("Message #launch")).unwrap();

	drop(screen);
	finish(fixture.owner);
}

#[test]
fn the_composer_sends_and_retries_messages() {
	let fixture = Fixture::new();
	let mut screen = fixture.mount(1280.0, 2200.0);

	screen.get(&Finder::text("Ship it on Thursday?")).unwrap();
	// A pending copy whose key the live list already holds is gone: the
	// server's row replaced it.
	assert_eq!(
		screen
			.get_all(&Finder::text("Thursday works"))
			.unwrap()
			.len(),
		1
	);
	screen.get(&Finder::text("On its way")).unwrap();
	screen.get(&Finder::text("Sending…")).unwrap();
	screen.get(&Finder::text("Refused")).unwrap();
	screen.get(&Finder::text("Not sent")).unwrap();

	// Retry sends the same key again, before a second failure makes the
	// button ambiguous.
	screen
		.tap(&Finder::label("Send this message again"))
		.unwrap();
	settle();
	screen.pump();

	// Typing tells the room, sending clears the draft and shows the copy.
	screen
		.enter_text(&Finder::label("Message #general"), "Friday works too")
		.unwrap();
	screen.tap(&Finder::text("Send")).unwrap();
	settle();
	screen.pump();
	screen.get(&Finder::text("Friday works too")).unwrap();
	assert_ne!(screen.get_all(&Finder::text("Not sent")).unwrap().len(), 0);

	// Clearing the field tells the room the person stopped.
	screen
		.enter_text(&Finder::label("Message #general"), "")
		.unwrap();
	settle();
	screen.pump();

	drop(screen);
	finish(fixture.owner);
}

#[test]
fn the_bell_lists_notices_and_runs_their_action() {
	let fixture = Fixture::new();
	// Keep the chat's own Retry button out of the tree: this test is about
	// the bell's.
	fixture.huddle().tab.set(Tab::Board);
	let notice = Notice::new("Your tip to Grace Hopper failed")
		.body("5 coins: The settlement service declined the tip (simulated)")
		.action(NoticeAction::new("Retry", RetryTip { tip: id(50) }))
		.kind(NoticeKind::Error);
	fixture.set(
		InboxArgs { limit: 50 },
		Inbox {
			notices: vec![
				InboxNotice {
					id: "notice-1".into(),
					notice,
					read: false,
					created_at: "2026-11-01T10:00:00Z".into(),
				},
				InboxNotice {
					id: "notice-2".into(),
					notice: Notice::new("An update with no action").action(NoticeAction {
						label: "Nothing here".into(),
						operation: "not_a_real_mutation".into(),
						arguments: serde_json::json!({}),
					}),
					read: true,
					created_at: "2026-11-01T09:00:00Z".into(),
				},
			],
			unread: 1,
		},
	);
	settle();
	let huddle = fixture.huddle();
	huddle.show_inbox.set(true);
	settle();

	let mut screen = fixture.mount(1280.0, 2200.0);

	// The bell says how many are unread, in words and in the label.
	screen.get(&Finder::text("Notices (1)")).unwrap();
	screen.get(&Finder::label("Notices, 1 unread")).unwrap();
	screen.get(&Finder::label("Notices")).unwrap();
	screen
		.get(&Finder::text("Your tip to Grace Hopper failed"))
		.unwrap();
	screen
		.get(&Finder::text(
			"5 coins: The settlement service declined the tip (simulated)",
		))
		.unwrap();

	// A notice's action only appears when its operation decodes to one the
	// app knows, and only unread notices offer a plain Read.
	screen.get(&Finder::text("Retry")).unwrap();
	assert!(screen.query_by_text("Nothing here").unwrap().is_none());
	screen
		.get(&Finder::label(
			"Mark “Your tip to Grace Hopper failed” read",
		))
		.unwrap();
	assert!(
		screen
			.query_by_label("Mark “An update with no action” read")
			.unwrap()
			.is_none(),
		"a notice that is already read offers no Read button"
	);

	screen.tap(&Finder::text("Retry")).unwrap();
	settle();
	screen.pump();
	screen.tap(&Finder::text("Mark all read")).unwrap();
	settle();
	screen.pump();

	// Nothing unread and nothing at all: the badge drops the count and the
	// panel explains what will appear.
	fixture.set(
		InboxArgs { limit: 50 },
		Inbox {
			notices: Vec::new(),
			unread: 0,
		},
	);
	settle();
	huddle.show_inbox.set(true);
	settle();
	screen.pump();
	screen.get(&Finder::label("Notices, none unread")).unwrap();
	screen
		.get(&Finder::text(
			"Nothing yet. Invites, reminders, and tips report here.",
		))
		.unwrap();

	// The bell toggles the panel, and says whether it is open.
	screen.tap(&Finder::test_id("bell")).unwrap();
	settle();
	screen.pump();
	assert!(screen.query_by_label("Notices").unwrap().is_none());
	screen.tap(&Finder::test_id("bell")).unwrap();
	settle();
	screen.pump();
	screen.get(&Finder::label("Notices")).unwrap();

	drop(screen);
	finish(fixture.owner);
}

#[test]
fn the_channel_header_shows_the_private_badge_summary_and_problem() {
	let fixture = Fixture::new();
	let huddle = fixture.huddle();
	huddle.selected.set(fixture.rooms.launch);
	huddle
		.problem
		.set(Some("The summary couldn't be saved.".into()));
	settle();
	let mut screen = fixture.mount(1280.0, 2200.0);

	screen.get(&Finder::text("#launch")).unwrap();
	screen.get(&Finder::text("Private")).unwrap();
	screen
		.get(&Finder::text("The summary couldn't be saved."))
		.unwrap();

	// Pressing summarize shows it running before it fails.
	screen.tap(&Finder::text("Summarize this channel")).unwrap();
	screen.pump();
	screen.get(&Finder::text("Summarizing…")).unwrap();
	settle();
	screen.pump();
	screen.get(&Finder::text("Summarize this channel")).unwrap();

	// The tabs choose the body beside the header.
	screen.tap(&Finder::text("Board")).unwrap();
	settle();
	screen.pump();
	screen.get(&Finder::text("To do · 0")).unwrap();
	screen.tap(&Finder::text("Activity")).unwrap();
	settle();
	screen.pump();
	screen.get(&Finder::text("Invite by email")).unwrap();

	screen.tap(&Finder::text("Chat")).unwrap();
	settle();
	screen.pump();

	// The general channel's pinned summary is shown as its own labelled
	// block.
	huddle.selected.set(fixture.rooms.general);
	huddle.problem.set(None);
	settle();
	screen.pump();
	screen.get(&Finder::label("Pinned summary")).unwrap();
	screen
		.get(&Finder::text("2 messages from 2 people in #general."))
		.unwrap();

	// With no channel open the header waits for the sidebar instead of
	// naming a channel it doesn't have.
	huddle.selected.set(no_channel());
	settle();
	screen.pump();
	screen.get(&Finder::text("Loading your channels…")).unwrap();

	drop(screen);
	finish(fixture.owner);
}

#[test]
fn the_top_bar_names_the_people_here_and_signs_out() {
	let fixture = Fixture::new();
	let mut screen = fixture.mount(1280.0, 1600.0);

	// Nobody has joined a live room here, so the preset label reads zero.
	// Its row has no width to show, so only the diagnostic tree holds it.
	assert!(screen.tree().contains("0 here now"));
	assert_ne!(screen.get_all(&Finder::text("Ada")).unwrap().len(), 0);
	screen.tap(&Finder::text("Sign out")).unwrap();
	settle();
	screen.pump();
	assert_ne!(screen.get_all(&Finder::text("Ada")).unwrap().len(), 0);

	drop(screen);
	finish(fixture.owner);
}
