//! The audit trail.

use leptoskit_db::Connection;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;

use crate::Account;
use crate::Auth;
use crate::AuthError;
use crate::Event;
use crate::EventKind;
use crate::NewEvent;
use crate::RequestInfo;
use crate::Result;
use crate::SecurityEvent;
use crate::SecurityEvents;
use crate::SignedIn;

/// The longest `detail` kept, in characters.
const DETAIL_LENGTH: usize = 200;

/// How many events a page holds unless it asks for fewer.
const PAGE: u32 = 50;

/// The most events one page holds.
const MOST: u32 = 100;

/// Records that `kind` happened to `account`, from `request`.
pub(crate) async fn record(
	connection: &impl Connection,
	account: Option<Id<Account>>,
	kind: EventKind,
	request: &RequestInfo,
) -> Result<()> {
	record_with(connection, account, kind, None, request).await
}

async fn record_with(
	connection: &impl Connection,
	account: Option<Id<Account>>,
	kind: EventKind,
	detail: Option<&str>,
	request: &RequestInfo,
) -> Result<()> {
	NewEvent {
		account_id: account,
		kind,
		detail: detail.map(|detail| detail.chars().take(DETAIL_LENGTH).collect()),
		ip: request.ip_text(),
		user_agent: request.user_agent_text(),
	}
	.insert(connection)
	.await?;
	Ok(())
}

impl Auth {
	/// Adds to the audit trail: `kind` happened to `account`, from
	/// `request`, concerning `detail`, such as the name of the app that was
	/// connected. For crates that build on sign-in, such as `leptoskit_idp`,
	/// to record their own events in `auth.events` next to sign-in's.
	///
	/// Pass the transaction that made the change, so the event and the
	/// change land together. `detail` is cut to 200 characters; never put a
	/// credential in it.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn record(
		&self,
		connection: &impl Connection,
		account: Option<Id<Account>>,
		kind: EventKind,
		detail: Option<&str>,
		request: &RequestInfo,
	) -> Result<()> {
		record_with(connection, account, kind, detail, request).await
	}

	/// What happened to `me`'s own account, newest first, for a security
	/// page: sign-ins, new devices, factors turned on and off, merges, apps
	/// connected, and the rest of the audit trail. Pages hold `limit` events
	/// (50 by default, at most 100); pass a page's `next` as `before` for
	/// the one after it.
	///
	/// The trail shows where someone signed in from, so it needs a session
	/// as fresh as changing a sign-in method does: step-up when the account
	/// has a second factor, and otherwise a sign-in within the step-up
	/// window.
	///
	/// # Errors
	///
	/// [`AuthError::Reauthenticate`] or [`AuthError::StepUpRequired`] when
	/// the session isn't fresh enough; [`AuthError::NotFound`] when `before`
	/// isn't a cursor this returned; or when the database fails.
	pub async fn security_events(
		&self,
		me: &SignedIn,
		before: Option<&str>,
		limit: Option<u32>,
	) -> Result<SecurityEvents> {
		self.require_fresh(&self.inner.db, me).await?;
		let before = before.map(Cursor::parse).transpose()?;
		let limit = limit.unwrap_or(PAGE).clamp(1, MOST);

		let mut events = Event::query()
			.filter(|event| {
				let mine = event.account_id.eq(me.account);
				match &before {
					Some(cursor) => {
						mine & (event.created_at.lt(cursor.at)
							| (event.created_at.eq(cursor.at) & event.id.lt(cursor.id)))
					}
					None => mine,
				}
			})
			.order_by(|event| event.created_at.desc())
			.order_by(|event| event.id.desc())
			.limit(u64::from(limit) + 1)
			.all(&self.inner.db)
			.await?;

		let more = events.len() > limit as usize;
		events.truncate(limit as usize);
		let next = more.then(|| events.last()).flatten().map(|last| {
			Cursor {
				at: last.created_at,
				id: last.id,
			}
			.to_string()
		});

		Ok(SecurityEvents {
			events: events.into_iter().map(SecurityEvent::from).collect(),
			next,
		})
	}

	/// Counts an attempt at `action` from `source` against `limit`, through
	/// the same rate limiter sign-in uses, for crates that build on it, such
	/// as `leptoskit_idp` counting guesses at device codes. The two are
	/// hashed together, apart from sign-in's own counters, so neither is
	/// stored.
	///
	/// # Errors
	///
	/// [`AuthError::RateLimited`](crate::AuthError::RateLimited) when the
	/// limit is used up, or when the limiter fails.
	pub async fn limit_attempts(
		&self,
		action: &str,
		source: &str,
		limit: crate::Limit,
	) -> Result<()> {
		let key = crate::secrets::digest(&[b"extension", action.as_bytes(), source.as_bytes()]);
		self.inner.limiter.hit(&key, limit).await
	}
}

/// Where a page of events ends: the last event's time and ID, which order
/// the trail together. Opaque to clients.
struct Cursor {
	at: Timestamp,
	id: Id<Event>,
}

impl Cursor {
	fn parse(text: &str) -> Result<Self> {
		let (millis, id) = text.split_once('.').ok_or(AuthError::NotFound)?;
		let at = millis
			.parse::<i64>()
			.ok()
			.and_then(Timestamp::from_timestamp_millis)
			.ok_or(AuthError::NotFound)?;
		let id = id.parse().map_err(|_| AuthError::NotFound)?;
		Ok(Self { at, id })
	}
}

impl std::fmt::Display for Cursor {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}.{}", self.at.timestamp_millis(), self.id)
	}
}

impl From<Event> for SecurityEvent {
	fn from(event: Event) -> Self {
		let device = event.user_agent.map(|user_agent| {
			RequestInfo {
				user_agent: Some(user_agent),
				..RequestInfo::default()
			}
			.device()
		});
		Self {
			kind: event.kind,
			detail: event.detail,
			device,
			ip: event.ip,
			at: event.created_at,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cursors_read_back_what_they_wrote() {
		let cursor = Cursor {
			at: Timestamp::from_timestamp_millis(1_793_489_828_123).unwrap(),
			id: Id::new(),
		};
		let text = cursor.to_string();
		let read = Cursor::parse(&text).unwrap();
		assert_eq!((read.at, read.id), (cursor.at, cursor.id));

		for broken in ["", "nope", "12.nope", "x.0193"] {
			assert!(Cursor::parse(broken).is_err(), "{broken}");
		}
	}
}
