//! The operation context for the task that is running it.
//!
//! A query or mutation can take `cx` as its first argument, or omit it and
//! call [`cx`] / [`mutation_cx`]. The value is the same context the server
//! passed in: policies, the read set, and (for a mutation) the transaction.

use std::future::Future;
use std::sync::Arc;

use super::MutationCx;
use super::QueryCx;
use crate::Error;

tokio::task_local! {
	static QUERY: Arc<QueryCx>;
	static MUTATION: Arc<MutationCx>;
}

/// The query this task is running.
///
/// ```ignore
/// #[rpc::query]
/// pub async fn notes(starting: Option<String>) -> rpc::Result<Vec<String>> {
///     let cx = rpc::cx()?;
///     let notes = Note::query().all(cx.db()).await?;
///     Ok(notes.into_iter().map(|note| note.title).collect())
/// }
/// ```
///
/// Hold the returned [`Arc`] while you use it. `rpc::cx()?.db()` drops the
/// context before the reference can be used.
///
/// # Errors
///
/// [`Error::Server`] when this task is not inside a query.
pub fn cx() -> Result<Arc<QueryCx>, Error> {
	QUERY
		.try_with(Arc::clone)
		.map_err(|_| Error::server("rpc::cx() outside a query"))
}

/// The mutation this task is running. Same rules as [`cx`].
///
/// # Errors
///
/// [`Error::Server`] when this task is not inside a mutation.
pub fn mutation_cx() -> Result<Arc<MutationCx>, Error> {
	MUTATION
		.try_with(Arc::clone)
		.map_err(|_| Error::server("rpc::mutation_cx() outside a mutation"))
}

pub(crate) async fn enter_query<F: Future>(cx: Arc<QueryCx>, future: F) -> F::Output {
	QUERY.scope(cx, future).await
}

pub(crate) async fn enter_mutation<F: Future>(cx: Arc<MutationCx>, future: F) -> F::Output {
	MUTATION.scope(cx, future).await
}
