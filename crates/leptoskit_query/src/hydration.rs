//! Dehydrating the cache to plain data and hydrating it back.
//!
//! Only scopes that opted in with
//! [`QueryScope::with_ssr`](crate::QueryScope::with_ssr) (or a custom codec)
//! take part. Successful data is always carried. A settled error is carried
//! too when the scope opted in with
//! [`QueryScope::with_ssr_error`](crate::QueryScope::with_ssr_error), so the
//! browser's first render matches an error the server already drew instead of
//! flashing a loading state. The browser refetches after that render.
//!
//! Hydration is lazy. A dehydrated entry whose query does not exist yet waits
//! in the cache and seeds the query the moment something creates it, so
//! `hydrate` can run before any component mounts.
//!
//! An entry finds its query by scope id and [key hash](crate::hash_key),
//! both of which are the same in every binary for a
//! [named](crate::QueryScope::with_name) scope, so the server's cache matches
//! the browser's and a persisted cache matches the next build's.

use serde::Deserialize;
use serde::Serialize;

use crate::key::ScopeId;

/// One query's data, ready to serialize.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DehydratedQuery {
	/// The scope's cache identity.
	pub scope: ScopeId,
	/// The scope's readable name, for debugging the payload.
	pub scope_name: String,
	/// The key as `Debug` printed it, for debugging the payload.
	pub key: String,
	/// The key's [stable hash](crate::hash_key), which is what hydration
	/// matches on.
	pub key_hash: u64,
	/// The data, encoded by the scope's codec. Absent when the server settled
	/// on an error and had no data to keep.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub data: Option<serde_json::Value>,
	/// When the data was fetched, in epoch milliseconds, so staleness
	/// carries across. Zero when only an error is carried.
	pub data_updated_at: u64,
	/// The error, encoded by the scope's error codec. Absent for a success,
	/// and for an error whose scope did not opt in with
	/// [`QueryScope::with_ssr_error`](crate::QueryScope::with_ssr_error).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error: Option<serde_json::Value>,
	/// When the error was fetched, in epoch milliseconds.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error_updated_at: Option<u64>,
}

impl DehydratedQuery {
	/// The later of the data and error timestamps, for deciding which copy
	/// of a query is newer.
	pub(crate) fn revision(&self) -> u64 {
		self.data_updated_at.max(self.error_updated_at.unwrap_or(0))
	}
}

/// A serializable snapshot of the successful, codec-enabled queries in a
/// cache.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DehydratedState {
	/// The dehydrated queries.
	pub queries: Vec<DehydratedQuery>,
}

/// Whether this code is running during server rendering.
///
/// True only inside a leptos SSR render: there is a hydration context and it
/// is not the browser's. Native targets, tests, and client-side rendering
/// all count as the client.
#[must_use]
pub fn is_server() -> bool {
	leptos::prelude::Owner::current_shared_context().is_some_and(|context| !context.is_browser())
}
