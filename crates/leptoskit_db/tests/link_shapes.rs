//! A link's own columns inside shapes, against both databases: `via =
//! tags_links` nests the link rows themselves, each able to nest the row it
//! points at, and a link to a row the request can't see isn't there.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db as db;
use leptoskit_db::Connection;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Shape;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;

/// Who reads: tags marked secret are hidden from everyone without it.
struct Insider;

/// A post, soft-deleted.
#[table(name = "posts")]
pub struct Post {
	pub id: Id<Self>,
	pub title: String,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A tag, some secret, some soft-deleted.
#[table(name = "tags")]
#[policy(select, |tag, cx| tag.secret.eq(false) | cx.global(|_: &Insider| Expr::always()))]
pub struct Tag {
	pub id: Id<Self>,
	pub name: String,
	pub secret: bool,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A tag on a post, in a position.
#[db::link(name = "post_tags", from = Post as tags, to = Tag as posts)]
pub struct PostTag {
	pub position: i16,
}

/// A post with its tags in order, and where each one is.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Post)]
struct PostCard {
	title: String,
	#[db(via = tags_links, order_by = |link| link.position.asc())]
	tags: Vec<Tagging>,
	#[db(count(tags_links))]
	tag_count: i64,
}

/// One tag on a post: the link's own column, and the tag.
#[derive(Shape, Debug, PartialEq)]
#[db(from = PostTag)]
struct Tagging {
	position: i16,
	#[db(via = tag)]
	tag: TagName,
}

/// A tag's name.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Tag)]
struct TagName {
	name: String,
}

/// The same, soft-deleted tags included.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Post)]
struct ArchiveCard {
	#[db(via = tags_links, with_deleted, order_by = |link| link.position.asc())]
	tags: Vec<ArchivedTagging>,
}

/// One tag on a post, whether or not the tag is deleted.
#[derive(Shape, Debug, PartialEq)]
#[db(from = PostTag)]
struct ArchivedTagging {
	position: i16,
	#[db(via = tag, with_deleted)]
	tag: TagName,
}

/// A tag with the posts carrying it, from the other end.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Tag)]
struct TagCard {
	name: String,
	#[db(via = posts_links)]
	posts: Vec<Carried>,
}

/// A post carrying a tag, and where.
#[derive(Shape, Debug, PartialEq)]
#[db(from = PostTag)]
struct Carried {
	position: i16,
	#[db(via = post)]
	post: PostTitle,
}

/// A post's title.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Post)]
struct PostTitle {
	title: String,
}

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

async fn tag(db: &Db, name: &str, secret: bool) -> Tag {
	NewTag {
		name: name.into(),
		secret,
	}
	.insert(db)
	.await
	.unwrap()
}

fn tagging(position: i16, name: &str) -> Tagging {
	Tagging {
		position,
		tag: TagName { name: name.into() },
	}
}

#[tokio::test]
async fn link_rows_carry_their_own_columns_into_shapes() {
	on_both(async |db| {
		let system = db.system("seeding");
		let post = NewPost {
			title: "Hello".into(),
		}
		.insert(&system)
		.await
		.unwrap();
		let rust = tag(&system, "rust", false).await;
		let db_tag = tag(&system, "db", false).await;
		let hidden = tag(&system, "hidden", true).await;
		let mut old = tag(&system, "old", false).await;

		for (tag, position) in [(&db_tag, 2), (&rust, 1), (&hidden, 0), (&old, 3)] {
			post.tags()
				.add(&system, tag.id, PostTagProperties { position })
				.await
				.unwrap();
		}

		old.delete(&system).await.unwrap();
		let card =
			|db: Db| async move { Post::query().shape::<PostCard>().one(&db).await.unwrap() };

		// Ordered by the link's own column; the secret tag and the
		// soft-deleted one aren't there, nor counted.
		assert_eq!(
			card(db.clone()).await,
			PostCard {
				title: "Hello".into(),
				tags: vec![tagging(1, "rust"), tagging(2, "db")],
				tag_count: 2,
			}
		);

		// An insider sees the secret tag, through the same policy.
		let insider = db.with_context(Context::new().with(Insider));
		assert_eq!(
			card(insider).await.tags,
			[tagging(0, "hidden"), tagging(1, "rust"), tagging(2, "db")]
		);

		// `with_deleted` brings the soft-deleted tag back.
		let archive = Post::query().shape::<ArchiveCard>().one(&db).await.unwrap();
		let positions: Vec<(i16, String)> = archive
			.tags
			.into_iter()
			.map(|tagging| (tagging.position, tagging.tag.name))
			.collect();
		assert_eq!(
			positions,
			[
				(1, "rust".to_string()),
				(2, "db".to_string()),
				(3, "old".to_string()),
			]
		);

		// From the other end: a soft-deleted post's link isn't there.
		let mut gone = NewPost {
			title: "Gone".into(),
		}
		.insert(&system)
		.await
		.unwrap();
		gone.tags()
			.add(&system, rust.id, PostTagProperties { position: 9 })
			.await
			.unwrap();
		gone.delete(&system).await.unwrap();
		let rust_card = Tag::query()
			.filter(|tag| tag.id.eq(rust.id))
			.shape::<TagCard>()
			.one(&db)
			.await
			.unwrap();
		assert_eq!(
			rust_card,
			TagCard {
				name: "rust".into(),
				posts: vec![Carried {
					position: 1,
					post: PostTitle {
						title: "Hello".into(),
					},
				}],
			}
		);
	})
	.await;
}

/// Tags on one post: each one's position, whether it's secret, and whether
/// it's deleted.
#[derive(Clone, Debug)]
struct Spec {
	tags: Vec<(i16, bool, bool)>,
	insider: bool,
}

fn spec() -> impl Strategy<Value = Spec> {
	(
		proptest::collection::vec((0..20_i16, any::<bool>(), any::<bool>()), 0..6),
		any::<bool>(),
	)
		.prop_map(|(tags, insider)| Spec { tags, insider })
}

async fn agrees_with_the_model(db: &Db, spec: Spec) -> Result<(), TestCaseError> {
	let system = db.system("resetting the property");
	Post::query().destroy(&system).await.unwrap();
	Tag::query().with_deleted().destroy(&system).await.unwrap();
	let post = NewPost {
		title: "post".into(),
	}
	.insert(&system)
	.await
	.unwrap();
	let mut expected = Vec::new();

	for (index, (position, secret, deleted)) in spec.tags.iter().enumerate() {
		let mut row = tag(&system, &format!("t{index}"), *secret).await;
		post.tags()
			.add(
				&system,
				row.id,
				PostTagProperties {
					position: *position,
				},
			)
			.await
			.unwrap();

		if *deleted {
			row.delete(&system).await.unwrap();
		}

		if !deleted && (!secret || spec.insider) {
			expected.push((*position, row.id, format!("t{index}")));
		}
	}

	// Ties in position come in the link's key order: the post, then the tag.
	expected.sort();
	let expected: Vec<Tagging> = expected
		.into_iter()
		.map(|(position, _, name)| tagging(position, &name))
		.collect();
	let request = if spec.insider {
		db.with_context(Context::new().with(Insider))
	} else {
		db.clone()
	};
	let card = Post::query()
		.shape::<PostCard>()
		.one(&request)
		.await
		.unwrap();
	prop_assert_eq!(card.tag_count, expected.len() as i64);
	prop_assert_eq!(card.tags, expected);
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
fn link_shapes_agree_with_the_model_on_sqlite() {
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect("sqlite::memory:").await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 96);
}

#[test]
fn link_shapes_agree_with_the_model_on_postgres() {
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
