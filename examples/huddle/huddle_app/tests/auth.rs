//! The sign-in screen's states, rendered headlessly with the real layout
//! engine and driven through its inputs: the email step, what it says when
//! the address is wrong or the server can't be reached, the passkey and
//! third-party buttons, the signed-in state, and the whole app's state
//! machine while a session comes and goes.
//!
//! The client can't reach a server in these tests, so every mutation ends in
//! the same refusal the app shows an offline native build. That is the point:
//! the tests cover the screen's own branches without a network.

// The sign-in screen mounts through an effect, and Leptos only runs effects
// in a client build (`csr`, which `--all-features` enables). The default
// feature set the example check builds leaves them inert, so these tests
// compile out there rather than assert on a screen that never mounts.
#![cfg(feature = "csr")]

mod support;

use leptoskit::auth::ProviderInfo;
use leptoskit::auth::WalletSettings;
use leptoskit::auth::client::WalletApp;
use leptoskit::auth::client::session;
use leptoskit::auth::client::wallet_apps;
use leptoskit::auth::client::wallet_settings;
use leptoskit::leptos::prelude::GetUntracked;
use leptoskit::leptos::prelude::Set;
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use support::finish;
use support::mount_auth;
use support::public_session;
use support::settle;
use support::sign_in_screen;

#[test]
fn the_email_step_checks_the_address_before_sending() {
	let (owner, sign_in, client) = sign_in_screen();
	client.set_query_data(&session(), (), None);
	settle();
	let mut screen = mount_auth(&owner, &sign_in, 800.0, 2400.0);

	// Before the mount effect runs the screen doesn't claim to know either
	// way, which keeps a server render and the browser's first render equal.
	assert!(!sign_in.is_signed_in());
	assert_eq!(sign_in.known(), Some(false));
	assert!(sign_in.session().is_none());

	screen
		.get(&Finder::text(
			"Enter your email address and we'll send you a code. If you're new, this creates your \
			 account.",
		))
		.unwrap();
	screen.get(&Finder::label("Email address")).unwrap();

	// A typo is caught here, before anything is sent.
	screen
		.enter_text(&Finder::label("Email address"), "ada")
		.unwrap();
	screen.tap(&Finder::text("Email me a code")).unwrap();
	settle();
	screen.pump();
	screen
		.get(&Finder::text(
			"Enter an email address, such as ada@example.com.",
		))
		.unwrap();

	// A real address reaches the client, which has no server here.
	screen
		.enter_text(&Finder::label("Email address"), "ada@example.com")
		.unwrap();
	screen.tap(&Finder::text("Email me a code")).unwrap();
	settle();
	screen.pump();
	assert!(support::refused(&screen), "the send's refusal is on screen");

	drop(screen);
	finish(owner);
}

#[test]
fn a_passkey_that_cannot_run_says_so_only_when_asked() {
	let (owner, sign_in, client) = sign_in_screen();
	client.set_query_data(&session(), (), None);
	settle();
	let mut screen = mount_auth(&owner, &sign_in, 800.0, 2400.0);

	// Asking for a passkey makes its refusal worth showing, rather than
	// staying quiet the way the autofill request does.
	screen.tap(&Finder::text("Sign in with a passkey")).unwrap();
	settle();
	screen.pump();
	assert!(
		support::refused(&screen),
		"the passkey's refusal is on screen"
	);

	drop(screen);
	finish(owner);
}

#[test]
fn providers_and_wallets_get_a_button_each() {
	let (owner, sign_in, client) = sign_in_screen();
	client.set_query_data(&session(), (), None);
	client.set_query_data(
		&leptoskit::auth::client::providers(),
		(),
		vec![ProviderInfo {
			id: "google".into(),
			name: "Google".into(),
		}],
	);
	client.set_query_data(
		&wallet_settings(),
		(),
		WalletSettings {
			enabled: true,
			sign_in: true,
			chain_id: "mainnet".into(),
		},
	);
	client.set_query_data(
		&wallet_apps(),
		(),
		vec![WalletApp {
			name: "Phantom".into(),
			icon: "data:image/svg+xml,".into(),
			sign_in: true,
		}],
	);
	settle();
	let mut screen = mount_auth(&owner, &sign_in, 800.0, 2400.0);

	screen.get(&Finder::text("Continue with Google")).unwrap();
	// The wallet succeeds the settings' check but fails this build's
	// unsupported adapter, which is what the screen has to explain.
	screen.get(&Finder::text("Sign in with Phantom")).unwrap();

	screen.tap(&Finder::text("Continue with Google")).unwrap();
	settle();
	screen.pump();
	assert!(
		support::refused(&screen),
		"the provider's refusal is on screen"
	);

	screen.tap(&Finder::text("Sign in with Phantom")).unwrap();
	settle();
	screen.pump();
	assert!(
		support::refused(&screen),
		"the wallet's refusal is on screen"
	);

	drop(screen);
	finish(owner);
}

#[test]
fn wallets_without_sign_in_get_no_buttons() {
	let (owner, sign_in, client) = sign_in_screen();
	client.set_query_data(&session(), (), None);
	client.set_query_data(
		&wallet_settings(),
		(),
		WalletSettings {
			enabled: true,
			sign_in: false,
			chain_id: "mainnet".into(),
		},
	);
	client.set_query_data(
		&wallet_apps(),
		(),
		vec![WalletApp {
			name: "Phantom".into(),
			icon: "data:image/svg+xml,".into(),
			sign_in: true,
		}],
	);
	settle();
	let screen = mount_auth(&owner, &sign_in, 800.0, 2400.0);

	assert!(
		screen
			.query_by_text("Sign in with Phantom")
			.unwrap()
			.is_none(),
		"wallets can't sign in here"
	);

	drop(screen);
	finish(owner);
}

#[test]
fn a_signed_in_person_sees_their_state_and_can_sign_out() {
	let (owner, sign_in, client) = sign_in_screen();
	client.set_query_data(&session(), (), Some(public_session()));
	settle();
	let mut screen = mount_auth(&owner, &sign_in, 800.0, 1600.0);

	assert!(sign_in.is_signed_in());
	assert_eq!(sign_in.known(), Some(true));
	screen.get(&Finder::text("You're signed in.")).unwrap();

	screen.tap(&Finder::text("Sign out")).unwrap();
	settle();
	screen.pump();
	// The device has no server to tell; the screen keeps the person signed
	// in until the session query changes.
	screen.get(&Finder::text("You're signed in.")).unwrap();

	drop(screen);
	finish(owner);
}

#[test]
fn an_unreadable_session_offers_another_try() {
	let (owner, sign_in, _) = sign_in_screen();
	settle();
	let mut screen = mount_auth(&owner, &sign_in, 800.0, 1600.0);

	// With retries off, the first refusal lands on the next turn of the
	// pool's queue.
	settle();
	screen.pump();
	assert!(support::refused(&screen), "the read's refusal is on screen");
	screen.tap(&Finder::text("Try again")).unwrap();
	settle();
	screen.pump();
	assert!(support::refused(&screen), "the retry refuses the same way");

	drop(screen);
	finish(owner);
}

#[test]
fn the_whole_app_starts_signed_out_and_follows_a_session() {
	let (owner, shell) = support::provide_shell();
	let client = owner.with(leptoskit::query::use_query_client);

	// Nothing signed in yet: the sign-in screen is what the app renders.
	assert!(shell.huddle.get_untracked().is_none());
	assert!(!shell.sign_in.is_signed_in());

	// A session that lands later builds the signed-in state under the app's
	// own owner, and losing it tears that state down again.
	owner.with(|| client.set_query_data(&session(), (), Some(public_session())));
	settle();
	assert!(shell.sign_in.is_signed_in());

	let huddle = shell.huddle.get_untracked().expect("signed in");
	owner.with(|| huddle.selected.set(support::rooms().general));
	settle();

	owner.with(|| client.set_query_data(&session(), (), None));
	settle();
	assert!(!shell.sign_in.is_signed_in());
	assert!(shell.huddle.get_untracked().is_none());

	finish(owner);
}

#[cfg(feature = "web")]
#[test]
fn the_development_buttons_sign_in_without_a_code() {
	let (owner, sign_in, client) = sign_in_screen();
	client.set_query_data(&session(), (), None);
	settle();
	let screen = mount_auth(&owner, &sign_in, 800.0, 3600.0);

	// The shortcuts sit under the card, below the fold: a person scrolls to
	// them, so the tree has them while the visible finders do not.
	let tree = screen.tree();
	assert!(tree.contains("Development"), "{tree}");
	assert!(tree.contains("Ada"), "{tree}");
	assert!(tree.contains("Grace"), "{tree}");
	assert!(tree.contains("Lin"), "{tree}");
	screen.get(&Finder::label("Email address")).unwrap();

	drop(screen);
	finish(owner);
}
