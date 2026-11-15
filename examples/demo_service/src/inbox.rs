//! The demo inbox: a bounded, in-memory [`Mailer`] that stands in for an
//! email app.
//!
//! The bounds are the design: at most 2,000 open addresses, at most 3
//! messages each, and a 10-minute lifetime pruned on every call — about
//! 14 MiB in the worst case, and nothing on disk but the receipt id and
//! time the mailer records durably on the person it mailed. Mail to an
//! address this process never opened is dropped and counted, so the
//! provider answers every address the same way and no route can list
//! anyone else's mail.
//!
//! Receipts are the honesty contract: the mailer records the same random
//! receipt id and time durably *and* locally before it reports success,
//! so a reader compares the persisted receipt with what is still held
//! here. A fresh message is never reported lost; a message another
//! process sent — or one this process lost — is reported unavailable,
//! without claiming which release caused it. No `Debug` or log output
//! ever carries a message's contents.

use std::collections::HashMap;
use std::collections::VecDeque;

use anyhow::Context as _;
use anyhow::Result;
use leptoskit_auth::Mail;
use leptoskit_auth::MailError;
use leptoskit_auth::MailFuture;
use leptoskit_auth::Mailer;
use leptoskit_db::Db;
use leptoskit_db::Email;
use leptoskit_db::Timestamp;

use crate::visitors::DemoPerson;

/// How long a delivered message stays readable: the provider's code
/// lifetime, so a message never outlives the code it carries.
pub(crate) const TTL: chrono::TimeDelta = chrono::TimeDelta::minutes(10);

/// The most addresses this process keeps open, evicting the least
/// recently opened first.
const MAX_ADDRESSES: usize = 2_000;

/// The most messages kept per address, oldest dropped first.
const MAX_MESSAGES: usize = 3;

/// The longest subject kept, in bytes.
const MAX_SUBJECT: usize = 200;

/// The longest body kept, in bytes.
const MAX_TEXT: usize = 2 * 1024;

/// How long a receipt id is, as hex characters.
const RECEIPT_LEN: usize = 16;

/// One delivered message, as this process holds it in memory.
#[derive(Clone)]
pub(crate) struct Stored {
	/// The opaque receipt id the mailer recorded durably and locally.
	pub receipt: String,
	/// When the provider handed the message to this mailer.
	pub sent_at: Timestamp,
	/// The subject, truncated to [`MAX_SUBJECT`] bytes.
	pub subject: String,
	/// The plain-text body, truncated to [`MAX_TEXT`] bytes.
	pub text: String,
}

/// One open address and the messages it still holds.
struct Entry {
	messages: VecDeque<Stored>,
	/// A clock counter: the highest `opened` wins eviction, so the map
	/// never needs an ordering structure to find the least recent.
	opened: u64,
	/// When the address was last opened: its liveness, independent of
	/// whether a message has arrived yet.
	opened_at: Timestamp,
}

struct State {
	addresses: HashMap<String, Entry>,
	/// The eviction counter's next value.
	ticks: u64,
	/// How many sends to unopened addresses this process has dropped.
	dropped: u64,
}

/// The bounded inbox behind the site's real email-code sign-in.
#[derive(Clone)]
pub(crate) struct DemoInbox {
	db: Db,
	state: std::sync::Arc<std::sync::Mutex<State>>,
}

impl DemoInbox {
	/// An empty inbox writing receipts through `db`.
	pub(crate) fn new(db: Db) -> Self {
		Self {
			db,
			state: std::sync::Arc::new(std::sync::Mutex::new(State {
				addresses: HashMap::new(),
				ticks: 0,
				dropped: 0,
			})),
		}
	}

	/// Opens `address` for this process: from now its mail is kept,
	/// within the bounds. Called only for addresses the caller proved
	/// this pass owns.
	pub(crate) fn open(&self, address: &str) {
		let at = now();
		let mut state = self.lock();
		prune(&mut state, at);
		open_address(&mut state, address, at);
	}

	/// The messages still held for `address`, oldest first.
	pub(crate) fn messages_for(&self, address: &str) -> Vec<Stored> {
		let mut state = self.lock();
		prune(&mut state, now());
		state
			.addresses
			.get(address)
			.map(|entry| entry.messages.iter().cloned().collect())
			.unwrap_or_default()
	}

	/// Whether a persisted latest receipt names a message this process
	/// cannot show: `Some(persona)` when the person's latest receipt is
	/// recent and no retained message carries its id.
	///
	/// The comparison is by receipt id, never by timestamp ordering: a
	/// send recorded here and a send recorded by a handover partner are
	/// told apart by what they wrote, not by which clock read later.
	pub(crate) fn unavailable(
		persona: crate::visitors::Persona,
		receipt: Option<&str>,
		sent_at: Option<Timestamp>,
		retained: &[Stored],
	) -> Option<crate::visitors::Persona> {
		let receipt = receipt?;
		let sent_at = sent_at?;
		let now = now();

		if sent_at + TTL <= now {
			// The message has expired everywhere, so there is nothing to
			// be unavailable: the honest answer is silence, and a resend
			// is the way forward.
			return None;
		}

		let known_locally = retained.iter().any(|stored| stored.receipt == receipt);

		(!known_locally).then_some(persona)
	}

	/// How many sends to unopened addresses this process dropped.
	#[cfg(test)]
	pub(crate) fn dropped(&self) -> u64 {
		self.lock().dropped
	}

	fn lock(&self) -> std::sync::MutexGuard<'_, State> {
		// A panic while holding the lock can't corrupt the map: the
		// guards only ever insert, remove, and truncate whole entries.
		self.state
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
	}

	/// Records `receipt` and `sent_at` durably on the person `address`
	/// belongs to. Returns the number of people claimed: zero means no
	/// person owns the address, which is a failure to record, not a
	/// success to gloss over.
	async fn record_receipt(
		db: &Db,
		address: &str,
		receipt: &str,
		sent_at: Timestamp,
	) -> Result<u64> {
		let Ok(email) = Email::parse(address) else {
			anyhow::bail!("no person owns {address}: it is not an address");
		};

		DemoPerson::update()
			.filter(|person| person.email.eq(email))
			.set(|person| person.last_receipt_id, Some(receipt.to_owned()))
			.set(|person| person.last_receipt_at, Some(sent_at))
			.run(db)
			.await
			.context("recording the demo mail receipt")
	}
}

impl Mailer for DemoInbox {
	fn send(&self, mail: Mail) -> MailFuture<'_> {
		Box::pin(async move {
			// The provider normalizes the address before sending; this
			// inbox keyed its opens the same way.
			let address = mail.to.trim().to_ascii_lowercase();

			// Only an address this process opened retains anything. The
			// content is dropped with the decision: it is never logged
			// and never stored.
			let opened = {
				let mut state = self.lock();
				prune(&mut state, now());
				state.addresses.contains_key(&address)
			};

			let sent_at = now();
			let receipt = receipt_id().map_err(|error| MailError(error.to_string()))?;

			// Durable first: if this fails — the database refused, or no
			// person owns the address — the visitor's code was
			// verifiably not deliverable, and the provider must say so
			// instead of promising a message nobody can show.
			let recorded = Self::record_receipt(&self.db, &address, &receipt, sent_at)
				.await
				.map_err(|error| MailError(error.to_string()))?;

			if recorded == 0 && opened {
				return Err(MailError("no demo person owns this address".into()));
			}

			if !opened {
				// Auth has already replaced any earlier challenge. Record a
				// known person's new receipt even when their inbox was evicted,
				// so retained mail in another process is not reported current.
				self.lock().dropped += 1;
				return Ok(());
			}

			let stored = Stored {
				receipt,
				sent_at,
				subject: truncate(&mail.subject, MAX_SUBJECT),
				text: truncate(&mail.text, MAX_TEXT),
			};

			// The address was just proven open; re-open it so a send
			// keeps a busy address alive against eviction.
			let mut state = self.lock();
			open_address(&mut state, &address, sent_at);

			let entry = state
				.addresses
				.get_mut(&address)
				.expect("open_address inserted the entry");
			entry.messages.push_back(stored);

			while entry.messages.len() > MAX_MESSAGES {
				entry.messages.pop_front();
			}

			Ok(())
		})
	}
}

impl std::fmt::Debug for DemoInbox {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		// Contents never reach Debug: a log line is a disk write, and
		// the messages carry the visitors' codes.
		let state = self.lock();
		f.debug_struct("DemoInbox")
			.field("open_addresses", &state.addresses.len())
			.field("dropped", &state.dropped)
			.finish_non_exhaustive()
	}
}

/// Refreshes `address` in the open set, evicting the least recently
/// opened address when the set would grow past its bound.
fn open_address(state: &mut State, address: &str, now: Timestamp) {
	state.ticks += 1;
	let tick = state.ticks;

	if let Some(entry) = state.addresses.get_mut(address) {
		entry.opened = tick;
		entry.opened_at = now;
		return;
	}

	state.addresses.insert(
		address.to_owned(),
		Entry {
			messages: VecDeque::new(),
			opened: tick,
			opened_at: now,
		},
	);

	while state.addresses.len() > MAX_ADDRESSES {
		let stalest = state
			.addresses
			.iter()
			.min_by_key(|(_, entry)| entry.opened)
			.map(|(address, _)| address.clone());
		let Some(stalest) = stalest else {
			break;
		};

		// The evicted address's mail goes with it; nothing is retained
		// for an address this process no longer holds.
		state.addresses.remove(&stalest);
	}
}

/// Drops every message past its lifetime, and every address whose last
/// open is itself stale and that has nothing left to show. Called on
/// every entry point, so the bounds hold without a background sweeper.
/// An address opened moments ago survives without messages: its send is
/// on its way.
fn prune(state: &mut State, now: Timestamp) {
	let horizon = now - TTL;

	state.addresses.retain(|_, entry| {
		entry.messages.retain(|stored| stored.sent_at > horizon);
		entry.opened_at > horizon || !entry.messages.is_empty()
	});
}

/// A random opaque receipt id: 64 bits of hex. It correlates one
/// delivered message with the durable record of it, and nothing else.
fn receipt_id() -> Result<String> {
	let mut bytes = [0_u8; RECEIPT_LEN / 2];
	getrandom::fill(&mut bytes).context("the OS random source failed")?;
	Ok(hex::encode(bytes))
}

/// Cuts `text` to at most `max` bytes without splitting a character, so
/// a truncated subject or body stays valid UTF-8.
fn truncate(text: &str, max: usize) -> String {
	if text.len() <= max {
		return text.to_owned();
	}

	let mut kept = String::new();

	for character in text.chars() {
		if kept.len() + character.len_utf8() > max {
			break;
		}

		kept.push(character);
	}

	kept
}

/// The mailer's clock. A seam, so the unit tests can hold time still.
fn now() -> Timestamp {
	leptoskit_db::now()
}

#[cfg(test)]
mod tests {
	use leptoskit_auth::Mail;
	use leptoskit_db::Email;
	use leptoskit_db::now;

	use super::*;
	use crate::visitors::NewDemoPass;
	use crate::visitors::NewDemoPerson;
	use crate::visitors::Persona;

	fn mail(to: &str, code: &str) -> Mail {
		Mail {
			to: to.to_owned(),
			subject: format!("Your Notes sign-in code: {code}"),
			text: format!("Your code is {code}."),
		}
	}

	/// An inbox over a database where `address` is owned by a real
	/// person row, the way every delivered send assumes.
	async fn inbox_with_person(address: &str) -> DemoInbox {
		let db = leptoskit_db::testing::sqlite().await;
		let system = db.system("demo: inbox test");
		let pass = NewDemoPass {
			token_hash: format!("hash-{address}"),
			ip_hash: None,
		}
		.insert(&system)
		.await
		.expect("the pass inserts");
		NewDemoPerson {
			pass_id: pass.id,
			persona: Persona::Ada,
			email: Email::parse(address).expect("a valid test address"),
			account_id: None,
			last_receipt_id: None,
			last_receipt_at: None,
		}
		.insert(&system)
		.await
		.expect("the person inserts");
		DemoInbox::new(system)
	}

	#[tokio::test]
	async fn a_send_to_an_unopened_address_is_dropped_and_counted() {
		let inbox = DemoInbox::new(
			leptoskit_db::testing::sqlite()
				.await
				.system("demo: inbox test"),
		);

		inbox
			.send(mail("stranger@demo.invalid", "CODE1"))
			.await
			.expect("a dropped send is not an error");

		assert_eq!(inbox.dropped(), 1);
		assert!(inbox.messages_for("stranger@demo.invalid").is_empty());
	}

	#[tokio::test]
	async fn a_delivered_send_records_its_receipt_durably_and_locally() {
		let address = "ada-0123456789abcdef@demo.invalid";
		let inbox = inbox_with_person(address).await;
		inbox.open(address);

		inbox.send(mail(address, "CODE1")).await.expect("delivered");

		let kept = inbox.messages_for(address);
		assert_eq!(kept.len(), 1);
		assert_eq!(kept[0].receipt.len(), RECEIPT_LEN, "an opaque id");
	}

	#[tokio::test]
	async fn a_dropped_send_invalidates_the_previous_mail_receipt() {
		let address = "ada-0123456789abcdef@demo.invalid";
		let inbox = inbox_with_person(address).await;
		inbox.open(address);
		inbox.send(mail(address, "OLD")).await.expect("delivered");
		let old_messages = inbox.messages_for(address);
		inbox.lock().addresses.remove(address);
		inbox.send(mail(address, "NEW")).await.expect("dropped");
		let person = DemoPerson::query()
			.filter(|person| person.email.eq(Email::parse(address).expect("email")))
			.first(&inbox.db)
			.await
			.expect("read")
			.expect("person");
		assert_eq!(inbox.dropped(), 1);
		assert!(inbox.messages_for(address).is_empty());
		assert_eq!(
			DemoInbox::unavailable(
				Persona::Ada,
				person.last_receipt_id.as_deref(),
				person.last_receipt_at,
				&old_messages,
			),
			Some(Persona::Ada),
			"older mail cannot stand in for a code that was dropped"
		);
	}

	#[tokio::test]
	async fn mail_to_an_opened_address_without_its_person_fails_honestly() {
		// No person row owns this address: the durable recording has
		// nowhere to write, so the mailer must report failure instead of
		// promising a message nobody can correlate later.
		let address = "orphan-0123456789abcdef@demo.invalid";
		let inbox = DemoInbox::new(
			leptoskit_db::testing::sqlite()
				.await
				.system("demo: inbox test"),
		);
		inbox.open(address);

		assert!(
			inbox.send(mail(address, "CODE1")).await.is_err(),
			"an unrecordable send is a mail failure"
		);
	}

	#[tokio::test]
	async fn an_opened_address_keeps_at_most_three_messages() {
		let address = "ada-0123456789abcdef@demo.invalid";
		let inbox = inbox_with_person(address).await;
		inbox.open(address);

		for code in ["CODE1", "CODE2", "CODE3", "CODE4"] {
			inbox.send(mail(address, code)).await.expect("delivered");
		}

		let kept = inbox.messages_for(address);
		assert_eq!(kept.len(), MAX_MESSAGES, "the oldest message drops");
		assert!(
			kept.iter().all(|stored| !stored.subject.contains("CODE1")),
			"CODE1 is the one that dropped"
		);
		assert!(kept[0].subject.contains("CODE2"), "the rest keep order");
	}

	#[tokio::test]
	async fn messages_expire_after_the_ttl() {
		let address = "grace-0123456789abcdef@demo.invalid";
		let inbox = inbox_with_person(address).await;
		inbox.open(address);
		inbox.send(mail(address, "CODE1")).await.expect("delivered");
		assert_eq!(inbox.messages_for(address).len(), 1);

		// Age the address and its message past the lifetime, the way
		// every call prunes.
		let aged = now() + TTL + chrono::TimeDelta::seconds(1);
		{
			let mut state = inbox.lock();
			let horizon = aged - TTL;
			if let Some(entry) = state.addresses.get_mut(address) {
				entry.opened_at = horizon;
				for stored in &mut entry.messages {
					stored.sent_at = horizon;
				}
			}
			prune(&mut state, aged);
		}

		assert!(
			inbox.messages_for(address).is_empty(),
			"expired is gone, address and all"
		);
	}

	#[tokio::test]
	async fn the_least_recently_opened_address_is_evicted() {
		let db = leptoskit_db::testing::sqlite().await;
		let system = db.system("demo: inbox test");
		let inbox = {
			// One pass per person: a pass owns each persona at most
			// once, so the eviction test's people live on their own
			// passes, exactly as minted ones do.
			for index in 0..=MAX_ADDRESSES {
				let address = address_of(index);
				let pass = NewDemoPass {
					token_hash: format!("hash-{index}"),
					ip_hash: None,
				}
				.insert(&system)
				.await
				.expect("the pass inserts");
				NewDemoPerson {
					pass_id: pass.id,
					persona: Persona::Ada,
					email: Email::parse(&address).expect("a valid test address"),
					account_id: None,
					last_receipt_id: None,
					last_receipt_at: None,
				}
				.insert(&system)
				.await
				.expect("the person inserts");
			}

			DemoInbox::new(system)
		};

		for index in 0..MAX_ADDRESSES {
			let address = address_of(index);
			inbox.open(&address);
			inbox.send(mail(&address, "CODE")).await.expect("delivered");
		}

		// The oldest address still holds its message.
		assert_eq!(inbox.messages_for(&address_of(0)).len(), 1);

		// Re-opening it makes it the newest, so the next open evicts a
		// different address instead.
		inbox.open(&address_of(0));
		inbox.open(&address_of(MAX_ADDRESSES));
		assert_eq!(
			inbox.messages_for(&address_of(0)).len(),
			1,
			"refreshed stays"
		);

		assert!(
			inbox.messages_for(&address_of(1)).is_empty(),
			"the least recently opened address went with its mail"
		);
	}

	fn address_of(index: usize) -> String {
		format!("p{index:04}-0123456789abcdef@demo.invalid")
	}

	#[tokio::test]
	async fn subjects_and_bodies_truncate_on_character_boundaries() {
		let long = "é".repeat(2_000);
		let truncated = truncate(&long, MAX_TEXT);
		assert!(truncated.len() <= MAX_TEXT);
		assert_eq!(truncated.chars().count(), MAX_TEXT / 2, "no split é");

		let subject = truncate(&"x".repeat(500), MAX_SUBJECT);
		assert_eq!(subject.len(), MAX_SUBJECT);
	}

	#[tokio::test]
	async fn unavailable_names_only_a_recent_receipt_absent_locally() {
		let persona = Persona::Ada;
		let retained = [Stored {
			receipt: "local".to_owned(),
			sent_at: now(),
			subject: "subject".to_owned(),
			text: "text".to_owned(),
		}];

		// A receipt this process still holds: available, never lost.
		assert_eq!(
			DemoInbox::unavailable(persona, Some("local"), Some(now()), &retained),
			None,
			"fresh mail is not marked lost"
		);

		// A recent receipt this process does not hold: unavailable.
		assert_eq!(
			DemoInbox::unavailable(persona, Some("other"), Some(now()), &retained),
			Some(persona)
		);

		// An old receipt: expired everywhere, so nothing to report.
		assert_eq!(
			DemoInbox::unavailable(persona, Some("other"), Some(now() - TTL), &retained),
			None
		);

		// No persisted receipt at all: nothing to report.
		assert_eq!(DemoInbox::unavailable(persona, None, None, &[]), None);
	}

	#[tokio::test]
	async fn debug_never_carries_contents() {
		let address = "ada-0123456789abcdef@demo.invalid";
		let inbox = inbox_with_person(address).await;
		inbox.open(address);
		inbox
			.send(mail(address, "SECRET-CODE"))
			.await
			.expect("sent");

		let rendered = format!("{inbox:?}");
		assert!(
			!rendered.contains("SECRET-CODE"),
			"Debug must not carry message contents: {rendered}"
		);
	}
}
