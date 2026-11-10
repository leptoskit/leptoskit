//! Orphan cleanup: `#[db::link(…, on_source_delete = delete_orphan_targets)]`
//! deletes a target once deleting its sources leaves no link to it, as
//! Gel's `on source delete delete target if orphan` does.
//!
//! The rules, which Gel's semantics and soft deletes together decide:
//!
//! - **The same link only.** A target is an orphan when no row of *this* link
//!   table links to it any more; other link tables, and other columns pointing
//!   at it, don't keep it.
//! - **Hard deletes are conservative.** Deleting a source for good removes a
//!   target for good only when no link row at all is left to it, even one from
//!   a soft-deleted source that could still be restored.
//! - **Soft deletes stay reversible.** Soft-deleting a source soft-deletes the
//!   targets no live link from a live source still reaches. A target table
//!   without a soft-delete column keeps its rows, since a delete that can be
//!   undone never removes data for good.
//! - **Integrity, not access.** Like cascades, cleanup reaches targets whatever
//!   the request may see. Each is deleted through the database layer, so its
//!   own hooks, audit log entries, live changes, cascades, and orphan rules
//!   apply, with the bypass recorded as the reason.
//!
//! Cleanup follows deletes the database layer makes of the source table:
//! `delete`, `destroy`, and bulk `Query::delete` and `Query::destroy`.

use leptoskit_db_schema::TableName;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use run::before;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use run::has_rules;

use crate::Link;
use crate::Table;

/// How the sources were deleted, which decides how their orphans are.
#[cfg_attr(
	not(any(feature = "postgres", feature = "sqlite")),
	expect(dead_code, reason = "only a driver deletes anything")
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
	/// Their `deleted_at` was set.
	Soft,
	/// They were removed for good.
	Hard,
}

/// A link whose targets are deleted when deleting their sources orphans
/// them. `#[db::link]` registers one per `on_source_delete =
/// delete_orphan_targets`.
#[doc(hidden)]
#[cfg_attr(
	not(any(feature = "postgres", feature = "sqlite")),
	expect(dead_code, reason = "only a driver deletes anything")
)]
pub struct OrphanRule {
	/// The link's `from` table, whose deletes leave orphans.
	source: fn() -> TableName,
	source_key: &'static str,
	/// The link table.
	link: fn() -> TableName,
	/// The link's column holding the source's key.
	from: &'static str,
	/// The link's column holding the target's key.
	to: &'static str,
	/// Whether the target can be soft-deleted, so a soft delete of a source
	/// reaches it.
	target_soft: bool,
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	clean: run::Clean,
}

impl OrphanRule {
	/// The rule for link `J`. Its rows are deleted on whichever thread the
	/// delete runs, so they must be shareable, as every generated row is.
	pub const fn of<J: Link>() -> Self
	where
		J::To: Send + Sync,
	{
		Self {
			source: <J::From as Table>::table_name,
			source_key: <J::From as Table>::KEY[0],
			link: J::table_name,
			from: J::FROM,
			to: J::TO,
			target_soft: <J::To as Table>::DELETED_AT.is_some(),
			#[cfg(any(feature = "postgres", feature = "sqlite"))]
			clean: run::clean::<J>,
		}
	}

	/// Whether deleting sources this way can orphan the rule's targets.
	#[cfg_attr(
		not(any(feature = "postgres", feature = "sqlite")),
		expect(dead_code, reason = "only a driver deletes anything")
	)]
	fn applies(&self, mode: Mode) -> bool {
		mode == Mode::Hard || self.target_soft
	}
}

inventory::collect!(OrphanRule);

/// Finding and deleting orphans, which needs a driver.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod run {
	use std::future::Future;
	use std::pin::Pin;

	use super::Mode;
	use super::OrphanRule;
	use crate::Col;
	use crate::Connection;
	use crate::Expr;
	use crate::Id;
	use crate::Link;
	use crate::Result;
	use crate::Table;
	use crate::Value;
	use crate::policy::Access;
	use crate::policy::Count;
	use crate::policy::Subquery;
	use crate::sql::Sql;

	/// The work that removes a link's orphans, given the candidate targets'
	/// keys, run on the transaction the sources were deleted in.
	pub(super) type Clean = for<'a> fn(
		&'a crate::Tx,
		Vec<Value>,
		Mode,
	)
		-> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

	/// Whether deleting rows of `T` may orphan another table's rows.
	pub(crate) fn has_rules<T: Table>() -> bool {
		let table = T::table_name();
		inventory::iter::<OrphanRule>().any(|rule| (rule.source)() == table)
	}

	/// The targets deleting some sources might orphan, read before the delete,
	/// while the links to them are still there.
	pub(crate) struct Pending {
		mode: Mode,
		candidates: Vec<(&'static OrphanRule, Vec<Value>)>,
	}

	/// Reads, for each orphan rule on `T`, the targets linked from the rows of
	/// `T` that `rows` matches, which are about to be deleted.
	pub(crate) async fn before<T: Table>(
		connection: &impl Connection,
		rows: &Expr,
		mode: Mode,
	) -> Result<Pending> {
		let table = T::table_name();
		let mut candidates = Vec::new();

		for rule in inventory::iter::<OrphanRule>()
			.filter(|rule| (rule.source)() == table && rule.applies(mode))
		{
			let mut sql = Sql::new(connection.dialect());
			sql.push("SELECT DISTINCT ")
				.identifier(rule.to)
				.push(" FROM ")
				.table_name(&(rule.link)())
				.push(" WHERE ")
				.identifier(rule.from)
				.push(" IN (SELECT ")
				.identifier(rule.source_key)
				.push(" FROM ");
			sql.enter(&table, "orphan");
			sql.push(" WHERE (");
			let written = rows.write(&mut sql);
			sql.leave();
			written?;
			sql.push("))");

			let keys: Vec<Value> = connection
				.fetch(sql.finish())
				.await?
				.into_iter()
				.filter_map(|row| row.into_iter().next())
				.collect();

			if !keys.is_empty() {
				candidates.push((rule, keys));
			}
		}

		Ok(Pending { mode, candidates })
	}

	impl Pending {
		/// Deletes the candidates the sources' delete left orphaned, on the
		/// transaction `connection` is.
		pub(crate) async fn clean(self, connection: &impl Connection) -> Result<()> {
			if self.candidates.is_empty() {
				return Ok(());
			}

			let Some(tx) = connection.current_transaction() else {
				return Err(crate::expr::invalid(
					"orphan cleanup runs only inside the delete's transaction",
				));
			};

			for (rule, keys) in self.candidates {
				(rule.clean)(tx, keys, self.mode).await?;
			}

			Ok(())
		}
	}

	/// The reason the audit log records for an orphan's delete.
	pub(crate) const REASON: &str = "a link's `on_source_delete = delete_orphan_targets`";

	/// Deletes the targets of link `J` among `keys` that nothing links to any
	/// more, as the rules above say.
	pub(super) fn clean<J: Link>(
		tx: &crate::Tx,
		keys: Vec<Value>,
		mode: Mode,
	) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>
	where
		J::To: Send + Sync,
	{
		Box::pin(async move {
			let integrity = Integrity::new(tx);
			let condition = orphaned::<J>(keys, mode);
			let rows = crate::write::select_for_update::<J::To>(&integrity, &condition).await?;

			for mut row in rows {
				match mode {
					Mode::Soft => crate::write::delete(&integrity, &mut row).await?,
					Mode::Hard => crate::write::destroy(&integrity, &row).await?,
				}
			}

			Ok(())
		})
	}

	/// The rows of `J::To` among `keys` that no link of `J` keeps: no link row
	/// at all after a hard delete; no live link from a live source after a soft
	/// one, and only targets not already soft-deleted.
	fn orphaned<J: Link>(keys: Vec<Value>, mode: Mode) -> Expr {
		let key = <J::To as Table>::KEY[0];
		let target = Col::<J, Id<J::To>>::new(J::TO).eq(Col::<J::To, Id<J::To>>::new(key));
		let links = match mode {
			Mode::Hard => Count::rows::<J>(Some(target), true),
			Mode::Soft => {
				let live_source = Expr::Within {
					column: J::FROM,
					selected: <J::From as Table>::KEY[0],
					rows: Box::new(Subquery::new::<J::From>(None, None, false)),
				};
				Count::rows::<J>(Some(target & live_source), false)
			}
		};
		let mut condition = Expr::In {
			column: key,
			values: keys,
		} & links.eq(0);

		if let (Mode::Soft, Some(column)) = (mode, <J::To as Table>::DELETED_AT) {
			condition = condition
				& Expr::Null {
					column,
					negated: false,
				};
		}

		condition
	}

	/// The transaction a delete runs in, acting as a recorded bypass: orphan
	/// cleanup is integrity, so it reaches targets whatever the request may
	/// see, and the audit log says why, and who asked.
	struct Integrity<'a> {
		tx: &'a crate::Tx,
		access: Access,
	}

	impl<'a> Integrity<'a> {
		fn new(tx: &'a crate::Tx) -> Self {
			Self {
				tx,
				access: Access::System {
					reason: REASON,
					actor: tx.access().actor(),
				},
			}
		}
	}

	impl Connection for Integrity<'_> {
		fn dialect(&self) -> crate::Dialect {
			self.tx.dialect()
		}

		fn access(&self) -> &Access {
			&self.access
		}

		fn row_security(&self) -> bool {
			self.tx.row_security()
		}

		fn fetch(
			&self,
			statement: crate::Statement,
		) -> impl Future<Output = Result<Vec<Vec<Value>>>> + Send {
			self.tx.fetch(statement)
		}

		fn execute(&self, statement: crate::Statement) -> impl Future<Output = Result<u64>> + Send {
			self.tx.execute(statement)
		}

		fn publish(
			&self,
			changes: Vec<crate::live::RowChange>,
		) -> impl Future<Output = Result<()>> + Send {
			self.tx.publish(changes)
		}

		async fn begin_unless_in_transaction(&self) -> Result<Option<crate::Tx>> {
			Ok(None)
		}

		fn current_transaction(&self) -> Option<&crate::Tx> {
			Some(self.tx)
		}
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		static RULE: OrphanRule = OrphanRule {
			source: || leptoskit_db_schema::TableName::new("posts"),
			source_key: "id",
			link: || leptoskit_db_schema::TableName::new("post_tags"),
			from: "post_id",
			to: "tag_id",
			target_soft: false,
			clean: |_, _, _| Box::pin(async { Ok(()) }),
		};

		#[tokio::test]
		async fn cleanup_runs_only_inside_a_transaction() {
			let db = crate::testing::sqlite().await;
			assert!(db.current_transaction().is_none());
			let pending = Pending {
				mode: Mode::Hard,
				candidates: vec![(&RULE, vec![Value::Int(1)])],
			};
			let refused = pending.clean(&db).await.unwrap_err();
			assert!(refused.to_string().contains("can't be stored"), "{refused}");

			let tx = db.begin().await.unwrap();
			assert!(tx.current_transaction().is_some());
			let integrity = Integrity::new(&tx);
			assert!(!integrity.row_security());
			assert_eq!(integrity.access().reason(), Some(REASON));
			let pending = Pending {
				mode: Mode::Hard,
				candidates: vec![(&RULE, vec![Value::Int(1)])],
			};
			pending.clean(&integrity).await.unwrap();
			tx.rollback().await.unwrap();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn soft_deletes_only_reach_targets_that_can_be_soft_deleted() {
		let rule = |target_soft| {
			OrphanRule {
				source: || TableName::new("posts"),
				source_key: "id",
				link: || TableName::new("post_tags"),
				from: "post_id",
				to: "tag_id",
				target_soft,
				#[cfg(any(feature = "postgres", feature = "sqlite"))]
				clean: |_, _, _| Box::pin(async { Ok(()) }),
			}
		};

		assert!(rule(false).applies(Mode::Hard));
		assert!(!rule(false).applies(Mode::Soft));
		assert!(rule(true).applies(Mode::Soft));
	}
}
