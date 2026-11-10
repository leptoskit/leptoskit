//! `on_source_delete = delete_orphan_targets` against both databases:
//! deleting a source deletes the targets it leaves unlinked, the same way
//! it was deleted, in the same transaction, through the database layer.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db as db;
use leptoskit_db::Actor;
use leptoskit_db::AfterDelete;
use leptoskit_db::AuditEntry;
use leptoskit_db::AuditOp;
use leptoskit_db::Connection;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;

/// A post, soft-deleted.
#[table(name = "posts")]
pub struct Post {
	pub id: Id<Self>,
	pub title: String,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A tag that exists only while a post carries it. No request may see
/// tags, so cleanup has to reach them whatever the policies say.
#[table(name = "tags", audit, hooks(after_delete))]
#[policy(select, |_, _| Expr::never())]
pub struct Tag {
	pub id: Id<Self>,
	pub name: String,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

impl AfterDelete for Tag {
	async fn after_delete(&self, tx: &impl Connection) -> db::Result<()> {
		if self.name == "sticky" {
			return Err(DbError::Forbidden {
				table: "tags".into(),
				message: "sticky tags stay".into(),
			});
		}

		NewTagDeletion {
			name: self.name.clone(),
			soft: self.deleted_at.is_some(),
		}
		.insert(tx)
		.await
		.map(|_| ())
	}
}

/// What tags' `after_delete` saw.
#[table(name = "tag_deletions")]
pub struct TagDeletion {
	pub id: Id<Self>,
	pub name: String,
	pub soft: bool,
}

/// A tag on a post; tags go with the last post that carries them.
#[db::link(name = "post_tags", from = Post as tags, to = Tag as posts, on_source_delete = delete_orphan_targets)]
pub struct PostTag {}

/// A note, which also links to tags; it doesn't keep a tag a post orphans.
/// Notes are removed for good, by anyone.
#[table(name = "notes")]
#[policy(all, |_, _| Expr::always())]
pub struct Note {
	pub id: Id<Self>,
}

/// A tag on a note; tags go with the last note too.
#[db::link(name = "note_tags", from = Note as tags, to = Tag as notes, on_source_delete = delete_orphan_targets)]
pub struct NoteTag {}

/// A color, hard-deleted only, owned by the tags that use it, and live.
#[table(name = "colors", live)]
pub struct Color {
	pub id: Id<Self>,
	pub name: String,
}

/// A color on a tag; colors go with the last tag, so cleanup chains.
#[db::link(name = "tag_colors", from = Tag as colors, to = Color as tags, on_source_delete = delete_orphan_targets)]
pub struct TagColor {}

async fn create_schema(db: &Db) {
	let dialect = db.dialect();
	let schema = leptoskit_db::schema(dialect).unwrap();
	let empty = Schema::empty(dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.statements.join(";\n"))
		.await
		.unwrap();
}

async fn on_both<F, Fut>(check: F)
where
	F: Fn(Db) -> Fut,
	Fut: Future<Output = ()>,
{
	let db = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&db).await;
	check(db).await;

	if let Some(cluster) = support::postgres::PostgresCluster::start() {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		check(db).await;
	}
}

async fn post(db: &Db, title: &str, tags: &[Id<Tag>]) -> Post {
	let post = NewPost {
		title: title.into(),
	}
	.insert(db)
	.await
	.unwrap();

	for tag in tags {
		post.tags().add(db, *tag, ()).await.unwrap();
	}

	post
}

async fn tag(db: &Db, name: &str) -> Id<Tag> {
	NewTag { name: name.into() }.insert(db).await.unwrap().id
}

/// Each tag's name and whether it's soft-deleted, for those still stored.
async fn tags(db: &Db) -> Vec<(String, bool)> {
	let mut tags: Vec<(String, bool)> = Tag::query()
		.with_deleted()
		.all(&db.system("reading tags"))
		.await
		.unwrap()
		.into_iter()
		.map(|tag| (tag.name, tag.deleted_at.is_some()))
		.collect();
	tags.sort();
	tags
}

fn owned(names: &[(&str, bool)]) -> Vec<(String, bool)> {
	let mut names: Vec<(String, bool)> = names
		.iter()
		.map(|(name, soft)| ((*name).to_string(), *soft))
		.collect();
	names.sort();
	names
}

#[tokio::test]
async fn soft_deleting_a_source_soft_deletes_the_targets_only_it_kept() {
	on_both(async |db| {
		let system = db.system("seeding");
		let [mine, shared, gone] = [
			tag(&system, "mine").await,
			tag(&system, "shared").await,
			tag(&system, "gone").await,
		];
		let mut first = post(&system, "first", &[mine, shared, gone]).await;
		post(&system, "second", &[shared]).await;
		let mut deleted = post(&system, "deleted", &[gone]).await;
		deleted.delete(&system).await.unwrap();
		// Deleting `deleted` left `gone` linked from `first`, a live post.
		assert_eq!(
			tags(&db).await,
			owned(&[("gone", false), ("mine", false), ("shared", false)])
		);

		// A note's link doesn't keep `mine`: only this link counts.
		let note = NewNote {}.insert(&system).await.unwrap();
		note.tags().add(&system, mine, ()).await.unwrap();

		let ada = Actor::from_uuid(uuid::Uuid::now_v7());
		first
			.delete(&db.with_context(Context::new().with(ada)))
			.await
			.unwrap();
		// `gone` is only linked from a soft-deleted post now, so it goes too.
		assert_eq!(
			tags(&db).await,
			owned(&[("gone", true), ("mine", true), ("shared", false)])
		);

		// Each went through the database layer: hooks ran, and the audit log
		// says why, and who asked.
		let mut seen: Vec<(String, bool)> = TagDeletion::query()
			.all(&db)
			.await
			.unwrap()
			.into_iter()
			.map(|deletion| (deletion.name, deletion.soft))
			.collect();
		seen.sort();
		assert_eq!(seen, owned(&[("gone", true), ("mine", true)]));
		let entries = AuditEntry::for_table::<Tag>()
			.filter(|entry| entry.op.eq(AuditOp::Delete))
			.all(&db.system("reading the log"))
			.await
			.unwrap();
		assert_eq!(entries.len(), 2);
		assert!(entries.iter().all(|entry| {
			entry.reason.as_deref() == Some("a link's `on_source_delete = delete_orphan_targets`")
				&& entry.actor() == Some(ada)
		}));
	})
	.await;
}

#[tokio::test]
async fn hard_deletes_remove_only_targets_nothing_links_to() {
	on_both(async |db| {
		let system = db.system("seeding");
		let [mine, shared, kept] = [
			tag(&system, "mine").await,
			tag(&system, "shared").await,
			tag(&system, "kept").await,
		];
		let red = NewColor { name: "red".into() }
			.insert(&system)
			.await
			.unwrap()
			.id;
		let blue = NewColor {
			name: "blue".into(),
		}
		.insert(&system)
		.await
		.unwrap()
		.id;
		let mine_row = Tag::query()
			.filter(|t| t.id.eq(mine))
			.one(&system)
			.await
			.unwrap();
		mine_row.colors().add(&system, red, ()).await.unwrap();
		let shared_row = Tag::query()
			.filter(|t| t.id.eq(shared))
			.one(&system)
			.await
			.unwrap();
		shared_row.colors().add(&system, blue, ()).await.unwrap();

		let first = post(&system, "first", &[mine, shared, kept]).await;
		post(&system, "second", &[shared]).await;
		// A soft-deleted post's link still keeps `kept` from a hard delete:
		// restoring the post would want it back.
		let mut sleeping = post(&system, "sleeping", &[kept]).await;
		sleeping.delete(&system).await.unwrap();

		first.destroy(&db).await.unwrap();
		assert_eq!(
			tags(&db).await,
			owned(&[("kept", false), ("shared", false)])
		);
		// `mine` went for good, and with it `red`, which only `mine` used.
		let colors: Vec<String> = Color::query()
			.all(&db)
			.await
			.unwrap()
			.into_iter()
			.map(|color| color.name)
			.collect();
		assert_eq!(colors, ["blue"]);
		assert_eq!(TagDeletion::query().all(&db).await.unwrap()[0].name, "mine");
	})
	.await;
}

#[tokio::test]
async fn bulk_deletes_clean_up_too() {
	on_both(async |db| {
		let system = db.system("seeding");
		let [a, b, c] = [
			tag(&system, "a").await,
			tag(&system, "b").await,
			tag(&system, "c").await,
		];
		post(&system, "one", &[a, b]).await;
		post(&system, "two", &[b]).await;
		post(&system, "three", &[c]).await;

		let deleted = Post::query()
			.filter(|p| p.title.ne("three"))
			.delete(&system)
			.await
			.unwrap();
		assert_eq!(deleted, 2);
		assert_eq!(
			tags(&db).await,
			owned(&[("a", true), ("b", true), ("c", false)])
		);

		let destroyed = Post::query().with_deleted().destroy(&system).await.unwrap();
		assert_eq!(destroyed, 3);
		assert!(tags(&db).await.is_empty(), "{:?}", tags(&db).await);
	})
	.await;
}

#[tokio::test]
async fn a_failing_cleanup_rolls_the_delete_back() {
	on_both(async |db| {
		let system = db.system("seeding");
		let sticky = tag(&system, "sticky").await;
		let mut post = post(&system, "post", &[sticky]).await;

		let refused = post.delete(&system).await.unwrap_err();
		assert_eq!(refused.to_string(), "sticky tags stay");
		assert!(Post::find(&db, post.id).await.unwrap().is_some());
		assert_eq!(tags(&db).await, owned(&[("sticky", false)]));

		// Inside the caller's transaction, the cleanup joins it: the refusal
		// fails the bulk delete, and the caller rolls everything back.
		let tx = system.begin().await.unwrap();
		NewPost {
			title: "in the transaction".into(),
		}
		.insert(&tx)
		.await
		.unwrap();
		assert!(Post::query().destroy(&tx).await.is_err());
		tx.rollback().await.unwrap();
		assert_eq!(Post::query().count(&db).await.unwrap(), 1);
	})
	.await;
}

/// A random set of posts, each with some of a few tags, then deletes.
#[derive(Clone, Debug)]
struct Spec {
	tags: usize,
	/// Each post's tags.
	posts: Vec<Vec<bool>>,
	/// Which posts to delete, in order, and whether for good.
	deletes: Vec<(usize, bool)>,
}

fn spec() -> impl Strategy<Value = Spec> {
	(1..5_usize, 1..5_usize).prop_flat_map(|(tags, posts)| {
		(
			proptest::collection::vec(proptest::collection::vec(any::<bool>(), tags), posts),
			proptest::collection::vec((0..posts, any::<bool>()), 0..6),
		)
			.prop_map(move |(posts, deletes)| {
				Spec {
					tags,
					posts,
					deletes,
				}
			})
	})
}

/// What the model says about a tag: gone, soft-deleted, or there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
	Live,
	Soft,
	Gone,
}

async fn agrees_with_the_model(db: &Db, spec: Spec) -> Result<(), TestCaseError> {
	let system = db.system("resetting the property");
	TagDeletion::query().destroy(&system).await.unwrap();
	Post::query().with_deleted().destroy(&system).await.unwrap();
	Tag::query()
		.with_deleted()
		.allow_skipping_hooks()
		.destroy(&system)
		.await
		.unwrap();

	let mut tag_ids = Vec::new();

	for index in 0..spec.tags {
		tag_ids.push(tag(&system, &format!("t{index}")).await);
	}

	let mut posts = Vec::new();

	for (index, carried) in spec.posts.iter().enumerate() {
		let tags: Vec<Id<Tag>> = carried
			.iter()
			.zip(&tag_ids)
			.filter(|(carries, _)| **carries)
			.map(|(_, id)| *id)
			.collect();
		posts.push(post(&system, &format!("p{index}"), &tags).await.id);
	}

	// The model: each post's state, and each tag's.
	let mut post_states = vec![State::Live; posts.len()];
	let mut tag_states = vec![State::Live; spec.tags];

	for (index, hard) in spec.deletes {
		if post_states[index] == State::Gone {
			continue;
		}

		let mut row = Post::query()
			.with_deleted()
			.filter(|p| p.id.eq(posts[index]))
			.one(&system)
			.await
			.unwrap();

		if hard {
			row.destroy(&system).await.unwrap();
		} else {
			row.delete(&system).await.unwrap();
		}

		post_states[index] = if hard { State::Gone } else { State::Soft };

		for (tag, state) in tag_states.iter_mut().enumerate() {
			if !spec.posts[index][tag] || *state == State::Gone {
				continue;
			}

			let linked_from = |wanted: &[State]| {
				spec.posts
					.iter()
					.zip(&post_states)
					.any(|(carried, post)| carried[tag] && wanted.contains(post))
			};

			if hard {
				if !linked_from(&[State::Live, State::Soft]) {
					*state = State::Gone;
				}
			} else if *state == State::Live && !linked_from(&[State::Live]) {
				*state = State::Soft;
			}
		}
	}

	let stored = Tag::query().with_deleted().all(&system).await.unwrap();

	for (index, id) in tag_ids.iter().enumerate() {
		let found = stored
			.iter()
			.find(|tag| tag.id == *id)
			.map_or(State::Gone, |tag| {
				if tag.deleted_at.is_some() {
					State::Soft
				} else {
					State::Live
				}
			});
		prop_assert_eq!(found, tag_states[index], "tag t{}", index);
	}

	Ok(())
}

fn run_property(db: &Db, runtime: &tokio::runtime::Runtime, cases: u32) {
	let mut runner = TestRunner::new(Config {
		cases,
		failure_persistence: None,
		..Config::default()
	});
	let outcome = runner.run(&spec(), |spec| {
		runtime.block_on(agrees_with_the_model(db, spec))
	});

	if let Err(error) = outcome {
		panic!("{error}");
	}
}

#[test]
fn cleanup_agrees_with_the_model_on_sqlite() {
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect("sqlite::memory:").await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 96);
}

#[test]
fn cleanup_agrees_with_the_model_on_postgres() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 48);
}

#[tokio::test]
async fn sources_without_soft_deletes_remove_their_orphans_for_good() {
	on_both(async |db| {
		let system = db.system("seeding");
		let [one, two, shared] = [
			tag(&system, "one").await,
			tag(&system, "two").await,
			tag(&system, "shared").await,
		];
		let first = NewNote {}.insert(&system).await.unwrap();
		let second = NewNote {}.insert(&system).await.unwrap();
		let third = NewNote {}.insert(&system).await.unwrap();
		first.tags().add(&system, one, ()).await.unwrap();
		second.tags().add(&system, two, ()).await.unwrap();
		second.tags().add(&system, shared, ()).await.unwrap();
		third.tags().add(&system, shared, ()).await.unwrap();

		// `delete` on a table without soft deletes removes for good.
		let mut first = first;
		first.delete(&db).await.unwrap();
		assert_eq!(tags(&db).await, owned(&[("shared", false), ("two", false)]));

		// So does a bulk delete, through the request's own policy.
		let deleted = Note::query()
			.filter(|note| note.id.eq(second.id))
			.delete(&db)
			.await
			.unwrap();
		assert_eq!(deleted, 1);
		assert_eq!(tags(&db).await, owned(&[("shared", false)]));
		let _ = third;
	})
	.await;
}
