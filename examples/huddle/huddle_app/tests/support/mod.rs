//! The harness the Huddle app tests share: each test thread's own task pool,
//! a query cache seeded straight from data instead of a server, and helpers
//! that mount the app's components with the real layout engine.
//!
//! Nothing here reaches a server. Queries are written into the cache the way
//! a live transition would write them, and every call a handler makes fails
//! the way an offline native app's does, so the tests are deterministic.

#![allow(dead_code)]

use std::cell::RefCell;

use futures::executor::LocalPool;
use futures::executor::LocalSpawner;
use futures::task::LocalSpawnExt as _;
use huddle_api::ChannelCard;
use huddle_api::InviteView;
use huddle_api::MessageView;
use huddle_api::Person;
use huddle_api::TaskCard;
use huddle_api::TipView;
use huddle_app::Shell;
use huddle_app::auth::SignIn;
use huddle_app::huddle::Huddle;
use huddle_app::huddle::Pending;
use leptoskit::auth::PublicSession;
use leptoskit::db::Id;
use leptoskit::leptos::prelude::*;
use leptoskit::query::QueryClient;
use leptoskit::query::QueryDefaults;
use leptoskit::query::RetryPolicy;
use leptoskit::query::use_query_client;
use leptoskit_rpc::client::Args;
use leptoskit_rpc::client::query_scope;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;

thread_local! {
	static POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
	static SPAWNER: LocalSpawner = POOL.with_borrow(LocalPool::spawner);
}

/// Each test thread's own task pool, run by [`settle`].
pub struct Pool;

impl any_spawner::CustomExecutor for Pool {
	fn spawn(&self, task: any_spawner::PinnedFuture<()>) {
		self.spawn_local(task);
	}

	fn spawn_local(&self, task: any_spawner::PinnedLocalFuture<()>) {
		// While the thread ends, the pool is gone and the task is dropped.
		let _ = SPAWNER.try_with(|spawner| spawner.spawn_local(task));
	}

	fn poll_local(&self) {}
}

/// Runs every task and effect that can make progress.
pub fn settle() {
	for _ in 0..4 {
		POOL.with_borrow_mut(LocalPool::run_until_stalled);
	}
}

/// Ends the test without dropping the tasks still waiting, such as the live
/// queries' subscriptions: they would run their cleanup after the thread's
/// reactive runtime is gone.
pub fn finish(owner: Owner) {
	std::mem::forget(owner);
	std::mem::forget(POOL.with_borrow_mut(std::mem::take));
}

/// A stable UUID for fixtures.
pub fn id<T>(n: u8) -> Id<T> {
	Id::parse(&format!("0192f3c4-0000-7000-8000-0000000000{n:02}")).expect("an ID")
}

/// Writes a query's answer into the cache as a live transition would.
pub fn seed<Q>(client: &QueryClient, args: Q, value: Q::Output)
where
	Q: leptoskit_rpc::Operation,
	Q::Output: Clone,
{
	client.set_query_data(&query_scope::<Q>(), Args::new(args), value);
}

/// A signed-in session as the auth endpoints return it, so the sign-in screen
/// can render its "You're signed in." state.
pub fn public_session() -> PublicSession {
	serde_json::from_value(serde_json::json!({
		"id": "0192f3c4-0000-7000-8000-000000000090",
		"account_id": "0192f3c4-0000-7000-8000-000000000091",
		"aal": "aal1",
		"authenticated_at": "2026-11-01T10:00:00Z",
		"mfa_at": null,
		"last_seen_at": "2026-11-01T10:00:00Z",
		"idle_expires_at": "2026-11-02T10:00:00Z",
		"expires_at": "2026-12-01T10:00:00Z",
		"device": "the test",
		"revoked_at": null,
		"created_at": "2026-11-01T10:00:00Z"
	}))
	.expect("a public session")
}

/// The people the fixtures use.
pub struct Cast {
	pub ada: Person,
	pub grace: Person,
	pub lin: Person,
}

impl Cast {
	pub fn new() -> Self {
		Self {
			ada: Person {
				id: id(10),
				name: "Ada".into(),
				is_admin: true,
			},
			grace: Person {
				id: id(11),
				name: "Grace Hopper".into(),
				is_admin: false,
			},
			lin: Person {
				id: id(12),
				name: "Lin".into(),
				is_admin: false,
			},
		}
	}

	pub fn everyone(&self) -> Vec<Person> {
		vec![self.ada.clone(), self.grace.clone(), self.lin.clone()]
	}
}

/// The two channels every signed-in fixture has: `#general`, owned by Ada,
/// with a pinned summary, and the private `#launch` with three unread.
pub struct Rooms {
	pub general: Id<huddle_db::Channel>,
	pub launch: Id<huddle_db::Channel>,
}

pub fn rooms() -> Rooms {
	Rooms {
		general: id(1),
		launch: id(2),
	}
}

pub fn channel_cards(rooms: &Rooms) -> Vec<ChannelCard> {
	vec![
		ChannelCard {
			id: rooms.general,
			name: "general".into(),
			private: false,
			owned: true,
			unread: 0,
			summary: Some("2 messages from 2 people in #general.".into()),
		},
		ChannelCard {
			id: rooms.launch,
			name: "launch".into(),
			private: true,
			owned: false,
			unread: 3,
			summary: None,
		},
	]
}

pub fn channel_card(
	id: Id<huddle_db::Channel>,
	name: &str,
	private: bool,
	owned: bool,
	unread: u64,
) -> ChannelCard {
	ChannelCard {
		id,
		name: name.into(),
		private,
		owned,
		unread,
		summary: None,
	}
}

pub fn message(n: u8, author: &Person, body: &str, key: &str) -> MessageView {
	MessageView {
		id: id(20 + n),
		author_id: author.id,
		author: author.name.clone(),
		body: body.into(),
		client_key: key.into(),
		sent_at: "2026-11-01T10:00:00Z".into(),
	}
}

pub fn task(n: u8, title: &str, column: &str, position: i64) -> TaskCard {
	TaskCard {
		id: id(30 + n),
		title: title.into(),
		column: column.into(),
		position,
	}
}

pub fn tip(n: u8, from: &Person, to: &Person, amount: i64, status: &str) -> TipView {
	TipView {
		id: id(50 + n),
		from_id: from.id,
		from: from.name.clone(),
		to_id: to.id,
		to: to.name.clone(),
		amount,
		status: status.into(),
		reason: (status == "failed")
			.then(|| "The settlement service declined the tip (simulated)".into()),
		receipt: (status == "confirmed").then(|| "sim-42".into()),
	}
}

pub fn invite(n: u8, email: &str, status: &str) -> InviteView {
	InviteView {
		id: id(40 + n),
		email: email.into(),
		status: status.into(),
	}
}

/// The signed-in app with no server: the queries hold the fixtures below and
/// every handler's call fails.
pub struct Fixture {
	pub owner: Owner,
	pub shell: Shell,
	pub client: QueryClient,
	pub cast: Cast,
	pub rooms: Rooms,
}

impl Fixture {
	/// Everything the signed-in screens show, for the common case.
	pub fn new() -> Self {
		let cast = Cast::new();
		let rooms = rooms();
		let owner = Owner::new();
		let (shell, client) = owner.with(|| {
			SPAWNER.with(|_| ());
			let _ = any_spawner::Executor::init_local_custom_executor(Pool);
			// One attempt: the tests' calls fail at once, so a screen's
			// refusal is on the next render rather than behind a retry
			// backoff the pool can't advance.
			QueryClient::new()
				.with_defaults(QueryDefaults::new().with_retry(RetryPolicy::Never))
				.provide();
			let huddle = Huddle::new();
			let client = use_query_client();

			seed(&client, huddle_api::Whoami {}, Some(cast.ada.clone()));
			seed(&client, huddle_api::People {}, cast.everyone());
			seed(&client, huddle_api::Channels {}, channel_cards(&rooms));
			seed(
				&client,
				huddle_api::Messages {
					channel: rooms.general,
					limit: huddle_api::PAGE,
					before: None,
				},
				vec![
					message(0, &cast.grace, "Ship it on Thursday?", "grace-1"),
					message(1, &cast.ada, "Thursday works", "ada-1"),
				],
			);
			seed(
				&client,
				huddle_api::Members {
					channel: rooms.general,
				},
				vec![cast.ada.clone(), cast.grace.clone()],
			);
			seed(
				&client,
				huddle_api::Board {
					channel: rooms.general,
				},
				vec![
					task(0, "Write the launch post", "todo", 0),
					task(1, "Review the copy", "todo", 1),
					task(2, "Fix sign-in", "doing", 0),
					task(3, "Ship it", "done", 0),
				],
			);
			seed(
				&client,
				huddle_api::Invites {
					channel: rooms.general,
				},
				vec![
					invite(0, "lin@huddle.dev", "sent"),
					invite(1, "pat@huddle.dev", "sending"),
					invite(2, "sam@huddle.dev", "failed"),
				],
			);
			seed(
				&client,
				huddle_api::Tips {},
				vec![
					tip(0, &cast.ada, &cast.grace, 5, "failed"),
					tip(1, &cast.ada, &cast.lin, 7, "confirmed"),
					tip(2, &cast.grace, &cast.ada, 3, "pending"),
					tip(3, &cast.lin, &cast.ada, 4, "submitted"),
				],
			);
			seed(
				&client,
				huddle_api::MySession {},
				huddle_api::SessionState { active: true },
			);

			huddle.selected.set(rooms.general);
			huddle.pending.set(vec![
				Pending {
					channel: rooms.general,
					client_key: "ada-2".into(),
					body: "On its way".into(),
					failed: false,
				},
				Pending {
					channel: rooms.general,
					client_key: "ada-3".into(),
					body: "Refused".into(),
					failed: true,
				},
				Pending {
					channel: rooms.launch,
					client_key: "ada-4".into(),
					body: "Another channel".into(),
					failed: false,
				},
			]);

			let shell = Shell {
				sign_in: SignIn::new(),
				huddle: RwSignal::new(Some(huddle)),
			};
			(shell, client)
		});
		settle();
		Self {
			owner,
			shell,
			client,
			cast,
			rooms,
		}
	}

	pub fn huddle(&self) -> Huddle {
		self.shell.huddle.get_untracked().expect("signed in")
	}

	/// Writes another answer into the cache, under the app's owner.
	pub fn set<Q>(&self, args: Q, value: Q::Output)
	where
		Q: leptoskit_rpc::Operation,
		Q::Output: Clone,
	{
		self.owner.with(|| seed(&self.client, args, value));
	}

	pub fn mount(&self, width: f32, height: f32) -> WidgetTester {
		mount_screen(&self.owner, &self.shell, width, height)
	}
}

/// Mounts the signed-in interface, as the app's own [`Screen`] would.
pub fn mount_screen(owner: &Owner, shell: &Shell, width: f32, height: f32) -> WidgetTester {
	let owner = owner.clone();
	let shell = *shell;
	WidgetTester::new(
		move || {
			owner.with(|| {
				let huddle = shell.huddle.get_untracked().expect("signed in");
				// The harness mounts the component in a builder call; the
				// screens themselves are the app's markup.
				// leptoskit-lint: allow(markup_style)
				huddle_app::ui::Screen(&shell, &huddle)
			})
		},
		width,
		height,
	)
	.expect("the screen mounts")
}

/// The refusals a sign-in call can leave on screen: a build with no client,
/// the server renderer's fallback, and the generic answer a failed session
/// read gets. Which one appears depends on which features the test binary
/// was built with, so a test asks whether one of them is showing.
pub const REFUSALS: [&str; 3] = [
	"This build can't reach the server: turn on leptoskit_auth's `web` feature for the browser, \
	 or `guest` for a native app.",
	"Signing in and out runs in the browser or the app, not while the server renders.",
	"Something went wrong; try again.",
];

/// Whether one of the refusals above is on screen.
pub fn refused(screen: &WidgetTester) -> bool {
	REFUSALS
		.iter()
		.any(|text| !screen.find_all(&Finder::text(*text)).is_empty())
}

/// Mounts the sign-in screen on its own, as [`huddle_app::Screen`] does while
/// nobody is signed in.
pub fn mount_auth(owner: &Owner, sign_in: &SignIn, width: f32, height: f32) -> WidgetTester {
	let owner = owner.clone();
	let sign_in = *sign_in;
	WidgetTester::new(
		// The harness mounts the component in a builder call, not a screen
		// of its own.
		// leptoskit-lint: allow(markup_style)
		move || owner.with(|| huddle_app::auth::Screen(sign_in)),
		width,
		height,
	)
	.expect("the sign-in screen mounts")
}

/// Starts the app's shared state with a fresh query cache, for tests that
/// drive [`huddle_app::provide`] and the whole [`huddle_app::Screen`]. The
/// caller settles it, so it can put a session in the cache before the
/// sign-in effect first looks.
pub fn provide_shell() -> (Owner, Shell) {
	let owner = Owner::new();
	let shell = owner.with(|| {
		SPAWNER.with(|_| ());
		let _ = any_spawner::Executor::init_local_custom_executor(Pool);
		huddle_app::provide()
	});
	(owner, shell)
}

/// A fresh sign-in screen with its own query cache.
pub fn sign_in_screen() -> (Owner, SignIn, QueryClient) {
	let owner = Owner::new();
	let (sign_in, client) = owner.with(|| {
		SPAWNER.with(|_| ());
		let _ = any_spawner::Executor::init_local_custom_executor(Pool);
		let client =
			QueryClient::new().with_defaults(QueryDefaults::new().with_retry(RetryPolicy::Never));
		client.provide();
		(SignIn::new(), client)
	});
	(owner, sign_in, client)
}
