//! The bodies of the audience catalogue: bounded counts, the walks over
//! the distinct names an index holds, and the audience estimate.
//!
//! Two reads carry everything here. **A capped count** counts the rows
//! one index range holds and stops at a cap, so its cost is the cap and
//! never the table. **A distinct walk** reads the distinct values of an
//! index's leading column one seek at a time — the next value greater
//! than the last — in a recursive statement, because neither engine skips
//! through an index on its own: `SELECT DISTINCT` reads every entry.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use leptoskit_db::Dialect;
use leptoskit_db::RawRead;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use leptoskit_rpc as rpc;
use uuid::Uuid;

use super::placeholder;
use crate::AppSend;
use crate::AudienceMember;
use crate::PersonAttribute;
use crate::Preference;
use crate::SendTarget;
use crate::TopicSubscription;
use crate::catalogue::AttributeKeySummary;
use crate::catalogue::AttributeValueSummary;
use crate::catalogue::AudienceEstimate;
use crate::catalogue::COUNT_CAP;
use crate::catalogue::CategorySummary;
use crate::catalogue::Count;
use crate::catalogue::ESTIMATE_WALK;
use crate::catalogue::EstimatedReach;
use crate::catalogue::MAX_CATALOGUE_PAGE;
use crate::catalogue::OVERVIEW_COUNT_CAP;
use crate::catalogue::OVERVIEW_TOPICS_CAP;
use crate::catalogue::Overview;
use crate::catalogue::PREFIX_SCAN;
use crate::catalogue::PlatformReach;
use crate::catalogue::TopicSummary;
use crate::push::PushEndpoint;
use crate::push::PushPlatform;

/// Every platform, in the order answers list them.
const PLATFORMS: [PushPlatform; 3] = [PushPlatform::Apns, PushPlatform::Fcm, PushPlatform::WebPush];

/// A column whose distinct values a walk reads: the leading column of an
/// index on a table — or the column after `scope`'s, when the walk stays
/// inside one value of the index's first column — and, with more than one
/// source, the union of several tables' columns.
#[derive(Clone, Debug)]
pub(crate) struct Names {
	/// Each source's table and column.
	sources: Vec<(&'static str, &'static str)>,
	scope: Option<(&'static str, String)>,
}

impl Names {
	/// The topics that have a subscriber: `topic_subscriptions (topic,
	/// person)`.
	pub(crate) fn topics() -> Self {
		Self {
			sources: vec![("topic_subscriptions", "topic")],
			scope: None,
		}
	}

	/// The attribute keys anyone holds: `person_attributes (key, value)`.
	pub(crate) fn attribute_keys() -> Self {
		Self {
			sources: vec![("person_attributes", "key")],
			scope: None,
		}
	}

	/// The values anyone holds for `key`: the same index, inside one key.
	pub(crate) fn attribute_values(key: &str) -> Self {
		Self {
			sources: vec![("person_attributes", "value")],
			scope: Some(("key", key.to_owned())),
		}
	}

	/// The categories of the sends in the history and of people's
	/// preferences, as one list: `sends (category, history_id)` and
	/// `category_preferences (category, opted_out)`.
	pub(crate) fn categories() -> Self {
		Self {
			sources: vec![("sends", "category"), ("category_preferences", "category")],
			scope: None,
		}
	}
}

/// The read of up to `most` distinct names after `after`, in the
/// database's order: one index seek per name. With `from`, the walk
/// starts at the first name not before it and ends at the first name that
/// doesn't start with it — which is only right where text is ordered by
/// its bytes, so the caller passes it on SQLite alone.
pub(crate) fn names_read(
	dialect: Dialect,
	names: &Names,
	after: Option<&str>,
	from: Option<&str>,
	most: u32,
) -> Statement {
	let mut args: Vec<Value> = Vec::new();
	let mut bind = |value: Value| {
		args.push(value);
		placeholder(dialect, args.len())
	};
	let scoped = names
		.scope
		.as_ref()
		.map(|(scope, value)| format!("\"{scope}\" = {} AND ", bind(Value::Text(value.clone()))))
		.unwrap_or_default();
	// The smallest name of one source that `bound` admits: one seek of
	// its index. A category of a send from before the history is NULL,
	// which no comparison admits.
	let seek = |(table, column): &(&str, &str), bound: &str| {
		format!(
			"(SELECT \"{column}\" FROM \"{table}\" WHERE {scoped}{} ORDER BY \"{column}\" ASC \
			 LIMIT 1)",
			bound.replace("{column}", &format!("\"{column}\""))
		)
	};
	// The smallest across the sources, compared by the database.
	let next = |bound: &str| {
		match names.sources.as_slice() {
			[only] => seek(only, bound),
			sources => {
				format!(
					"(SELECT min(\"n\") FROM ({}) AS \"nexts\")",
					sources
						.iter()
						.map(|source| format!("SELECT {} AS \"n\"", seek(source, bound)))
						.collect::<Vec<_>>()
						.join(" UNION ALL ")
				)
			}
		}
	};
	let mut first = String::from("{column} IS NOT NULL");

	if let Some(after) = after {
		first.push_str(" AND {column} > ");
		first.push_str(&bind(Value::Text(after.to_owned())));
	}

	let mut going = String::from("walk.\"name\" IS NOT NULL");

	if let Some(from) = from {
		let from = bind(Value::Text(from.to_owned()));
		first = format!("{first} AND {{column}} >= {from}");
		going = format!("{going} AND substr(walk.\"name\", 1, length({from})) = {from}");
	}

	let most = bind(Value::Int(i64::from(most)));
	let sql = format!(
		"WITH RECURSIVE walk(\"name\", \"n\") AS (SELECT {}, 1 UNION ALL SELECT {}, walk.\"n\" + \
		 1 FROM walk WHERE {going} AND walk.\"n\" < {most}) SELECT \"name\" FROM walk WHERE \
		 \"name\" IS NOT NULL",
		next(&first),
		next("{column} > walk.\"name\""),
	);

	Statement::new(sql, args)
}

/// One page of distinct names.
struct NamePage {
	names: Vec<String>,
	/// The last name the walk examined, when it found any: where the next
	/// page starts, whether or not that name is in `names`.
	end: Option<String>,
	has_more: bool,
}

/// Reads one page of `names`, keeping those that start with `prefix`.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] for a cursor that isn't a name, and whatever
/// the read fails with.
async fn names_page(
	db: &impl leptoskit_db::Read,
	names: &Names,
	reading: impl FnOnce(Statement) -> RawRead,
	page: &rpc::PageArgs,
	prefix: Option<&str>,
) -> rpc::Result<NamePage> {
	let cursor = |text: &str| {
		serde_json::from_str::<String>(text)
			.map_err(|_| rpc::Error::invalid(format!("`{text}` isn't a cursor of this query")))
	};
	let after = page.after.as_deref().map(cursor).transpose()?;
	let until = page.until.as_deref().map(cursor).transpose()?;
	let limit = page.limit.clamp(1, MAX_CATALOGUE_PAGE);
	let prefix = prefix.filter(|prefix| !prefix.is_empty());
	let dialect = db.dialect();
	// Where text is ordered by its bytes the names sharing a prefix sit
	// together, and the walk reads only them. Elsewhere it reads a
	// bounded run of names and keeps the ones that match.
	let from = prefix.filter(|_| dialect == Dialect::Sqlite);
	let most = match (prefix, from, until.as_ref()) {
		(Some(_), None, _) => PREFIX_SCAN,
		// A pinned page reads up to where it ended, and no further than
		// a bounded number of names.
		(_, _, Some(_)) => 5 * MAX_CATALOGUE_PAGE,
		// One past the page tells whether more follow — and one more for
		// the name that ends a prefix walk.
		_ => limit + 2,
	};
	let rows = reading(names_read(dialect, names, after.as_deref(), from, most))
		.fetch(db)
		.await?;
	let examined: Vec<String> = rows
		.into_iter()
		.filter_map(|row| {
			match row.into_iter().next() {
				Some(Value::Text(name)) => Some(name),
				_ => None,
			}
		})
		.collect();
	let exhausted = u32::try_from(examined.len()).unwrap_or(u32::MAX) < most;
	let mut found = Vec::new();
	let mut end = None;
	let mut has_more = !exhausted;

	for name in examined {
		let matches = prefix.is_none_or(|prefix| name.starts_with(prefix));

		if !matches && from.is_some() {
			// Past the prefix's run: nothing later matches.
			has_more = false;
			break;
		}

		if until.is_none() && u32::try_from(found.len()).unwrap_or(u32::MAX) == limit {
			if matches {
				has_more = true;
				break;
			}

			// Not a match and the page is full: keep looking for one,
			// so `has_more` is only said when it is true.
			continue;
		}

		end = Some(name.clone());

		if matches {
			found.push(name.clone());
		}

		if until.as_deref() == Some(name.as_str()) {
			has_more = true;
			break;
		}
	}

	Ok(NamePage {
		names: found,
		end,
		has_more,
	})
}

/// A page's cursor: the JSON of its last name, as the typed pages write
/// theirs.
fn cursor_of(end: Option<String>) -> Option<String> {
	end.map(|name| serde_json::to_string(&name).expect("a string is JSON"))
}

/// The read of how many rows of `table` match `conditions`, counted up to
/// `cap` and one more.
pub(crate) fn capped_count_read(
	dialect: Dialect,
	table: &str,
	conditions: &[(&str, Value)],
	extra: &str,
	distinct: Option<&str>,
	cap: u32,
) -> Statement {
	let mut args: Vec<Value> = Vec::new();
	let mut clauses: Vec<String> = conditions
		.iter()
		.map(|(column, value)| {
			args.push(value.clone());
			format!("\"{column}\" = {}", placeholder(dialect, args.len()))
		})
		.collect();

	if !extra.is_empty() {
		clauses.push(extra.to_owned());
	}

	let filter = if clauses.is_empty() {
		String::new()
	} else {
		format!(" WHERE {}", clauses.join(" AND "))
	};
	let selected =
		distinct.map_or_else(|| "1".to_owned(), |column| format!("DISTINCT \"{column}\""));
	args.push(Value::Int(i64::from(cap) + 1));

	Statement::new(
		format!(
			"SELECT count(*) FROM (SELECT {selected} FROM \"{table}\"{filter} LIMIT {}) AS capped",
			placeholder(dialect, args.len())
		),
		args,
	)
}

/// Runs a capped count.
async fn capped<T: leptoskit_db::Table>(
	db: &impl leptoskit_db::Read,
	statement: Statement,
	cap: u32,
) -> rpc::Result<Count> {
	let rows = statement.reads::<T>().fetch(db).await?;
	let counted = super::count_of(rows.first().and_then(|row| row.first()));

	Ok(Count {
		count: counted.min(cap),
		exact: counted <= cap,
	})
}

/// The read of how many people subscribe to `topic`, up to the cap.
pub(crate) fn topic_count_read(dialect: Dialect, topic: &str) -> Statement {
	capped_count_read(
		dialect,
		"topic_subscriptions",
		&[("topic", Value::Text(topic.to_owned()))],
		"",
		None,
		COUNT_CAP,
	)
}

/// The read of how many people hold `key`, or `value` for it, up to the
/// cap.
pub(crate) fn attribute_count_read(dialect: Dialect, key: &str, value: Option<&str>) -> Statement {
	let mut conditions = vec![("key", Value::Text(key.to_owned()))];

	if let Some(value) = value {
		conditions.push(("value", Value::Text(value.to_owned())));
	}

	capped_count_read(
		dialect,
		"person_attributes",
		&conditions,
		"",
		None,
		COUNT_CAP,
	)
}

/// The read of how many people opted out of `category`, up to the cap.
pub(crate) fn opted_out_count_read(dialect: Dialect, category: &str) -> Statement {
	capped_count_read(
		dialect,
		"category_preferences",
		&[
			("category", Value::Text(category.to_owned())),
			("opted_out", Value::Bool(true)),
		],
		"",
		None,
		COUNT_CAP,
	)
}

/// The read of how many people hold a live endpoint on `platform`, up to
/// the overview's cap.
pub(crate) fn platform_people_read(dialect: Dialect, platform: PushPlatform) -> Statement {
	capped_count_read(
		dialect,
		"push_endpoints",
		&[("platform", Value::Text(platform.as_str().to_owned()))],
		"\"invalidated_at\" IS NULL",
		Some("person"),
		OVERVIEW_COUNT_CAP,
	)
}

/// The body of
/// [`notifications_overview`](crate::catalogue::notifications_overview).
///
/// # Errors
///
/// Whatever a read fails with.
pub(crate) async fn overview(cx: &rpc::QueryCx) -> rpc::Result<Overview> {
	let db = cx.db();
	let dialect = db.dialect();
	let runtime = cx
		.get::<super::push::Push>()
		.and_then(|push| push.runtime.as_ref());
	let people_known = capped::<AudienceMember>(
		db,
		capped_count_read(
			dialect,
			"audience_members",
			&[],
			"",
			None,
			OVERVIEW_COUNT_CAP,
		),
		OVERVIEW_COUNT_CAP,
	)
	.await?;
	let mut people_with_push = Vec::with_capacity(PLATFORMS.len());

	for platform in PLATFORMS {
		people_with_push.push(PlatformReach {
			platform,
			people: capped::<PushEndpoint>(
				db,
				platform_people_read(dialect, platform),
				OVERVIEW_COUNT_CAP,
			)
			.await?,
			configured: runtime.is_some_and(|runtime| runtime.configured(platform)),
		});
	}

	let topics = names_read(
		dialect,
		&Names::topics(),
		None,
		None,
		OVERVIEW_TOPICS_CAP + 1,
	)
	.reads::<TopicSubscription>()
	.fetch(db)
	.await?
	.len();
	let topics = u32::try_from(topics).unwrap_or(u32::MAX);

	Ok(Overview {
		people_known,
		people_with_push,
		topics: Count {
			count: topics.min(OVERVIEW_TOPICS_CAP),
			exact: topics <= OVERVIEW_TOPICS_CAP,
		},
		open_topics: cx
			.get::<super::config::OpenTopics>()
			.map(|open| open.patterns().to_vec())
			.unwrap_or_default(),
	})
}

/// The body of [`topics`](crate::catalogue::topics).
///
/// # Errors
///
/// As [`names_page`].
pub(crate) async fn topics(
	cx: &rpc::QueryCx,
	page: &rpc::PageArgs,
	prefix: Option<&str>,
) -> rpc::Result<rpc::Page<TopicSummary>> {
	let db = cx.db();
	let open = cx.get::<super::config::OpenTopics>();
	let found = names_page(
		db,
		&Names::topics(),
		Statement::reads::<TopicSubscription>,
		page,
		prefix,
	)
	.await?;
	let mut items = Vec::with_capacity(found.names.len());

	for topic in found.names {
		items.push(TopicSummary {
			subscribers: capped::<TopicSubscription>(
				db,
				topic_count_read(db.dialect(), &topic),
				COUNT_CAP,
			)
			.await?,
			open: open.is_some_and(|open| open.is_open(&topic)),
			topic,
		});
	}

	Ok(rpc::Page {
		items,
		end: cursor_of(found.end),
		has_more: found.has_more,
	})
}

/// The body of [`attribute_keys`](crate::catalogue::attribute_keys).
///
/// # Errors
///
/// As [`names_page`].
pub(crate) async fn attribute_keys(
	db: &impl leptoskit_db::Read,
	page: &rpc::PageArgs,
) -> rpc::Result<rpc::Page<AttributeKeySummary>> {
	let found = names_page(
		db,
		&Names::attribute_keys(),
		Statement::reads::<PersonAttribute>,
		page,
		None,
	)
	.await?;
	let mut items = Vec::with_capacity(found.names.len());

	for key in found.names {
		items.push(AttributeKeySummary {
			people: capped::<PersonAttribute>(
				db,
				attribute_count_read(db.dialect(), &key, None),
				COUNT_CAP,
			)
			.await?,
			key,
		});
	}

	Ok(rpc::Page {
		items,
		end: cursor_of(found.end),
		has_more: found.has_more,
	})
}

/// The body of [`attribute_values`](crate::catalogue::attribute_values).
///
/// # Errors
///
/// As [`names_page`].
pub(crate) async fn attribute_values(
	db: &impl leptoskit_db::Read,
	key: &str,
	page: &rpc::PageArgs,
	prefix: Option<&str>,
) -> rpc::Result<rpc::Page<AttributeValueSummary>> {
	let found = names_page(
		db,
		&Names::attribute_values(key),
		Statement::reads::<PersonAttribute>,
		page,
		prefix,
	)
	.await?;
	let mut items = Vec::with_capacity(found.names.len());

	for value in found.names {
		items.push(AttributeValueSummary {
			people: capped::<PersonAttribute>(
				db,
				attribute_count_read(db.dialect(), key, Some(&value)),
				COUNT_CAP,
			)
			.await?,
			value,
		});
	}

	Ok(rpc::Page {
		items,
		end: cursor_of(found.end),
		has_more: found.has_more,
	})
}

/// The body of [`categories`](crate::catalogue::categories): the
/// categories of the sends in the history and of people's preferences,
/// as one list in name order.
///
/// # Errors
///
/// As [`names_page`].
pub(crate) async fn categories(
	db: &impl leptoskit_db::Read,
	page: &rpc::PageArgs,
) -> rpc::Result<rpc::Page<CategorySummary>> {
	let found = names_page(
		db,
		&Names::categories(),
		|statement| statement.reads::<AppSend>().reads::<Preference>(),
		page,
		None,
	)
	.await?;
	let mut items = Vec::with_capacity(found.names.len());

	for category in found.names {
		let last_sent = AppSend::query()
			.filter(|send| send.category.eq(category.clone()) & send.history_id.is_not_null())
			.order_by(|send| send.history_id.desc())
			.first(db)
			.await?
			.map(|send| send.created_at);

		items.push(CategorySummary {
			opted_out: capped::<Preference>(
				db,
				opted_out_count_read(db.dialect(), &category),
				COUNT_CAP,
			)
			.await?,
			last_sent,
			category,
		});
	}

	Ok(rpc::Page {
		items,
		end: cursor_of(found.end),
		has_more: found.has_more,
	})
}

/// The body of [`estimate_audience`](crate::catalogue::estimate_audience).
///
/// # Errors
///
/// Whatever a read fails with.
pub(crate) async fn estimate(
	db: &impl leptoskit_db::Read,
	target: &SendTarget,
	category: Option<&str>,
) -> rpc::Result<AudienceEstimate> {
	let mut estimate = AudienceEstimate {
		exact: true,
		..AudienceEstimate::default()
	};
	let mut reach: BTreeMap<&'static str, (PushPlatform, u32)> = BTreeMap::new();

	if let SendTarget::Recipients(named) = target {
		let people: Vec<Uuid> = named
			.iter()
			.copied()
			.collect::<BTreeSet<_>>()
			.into_iter()
			.collect();
		count_page(db, None, &people, None, category, &mut estimate, &mut reach).await?;
	} else {
		let walk = super::walk_of(target)?;
		let mut cursor: Option<Uuid> = None;
		let mut walked = 0;

		loop {
			let page = super::candidates(db, &walk, cursor, super::FANOUT_PAGE + 1).await?;

			if page.is_empty() {
				break;
			}

			let more = page.len() > super::FANOUT_PAGE;
			let page = &page[..page.len().saturating_sub(usize::from(more))];
			cursor = page.last().copied();
			walked += page.len();

			let verdict = super::evaluate(db, &walk, page).await?;
			count_page(
				db,
				walk.condition(),
				&verdict.matching,
				Some(&verdict.facts),
				category,
				&mut estimate,
				&mut reach,
			)
			.await?;

			if !more {
				break;
			}

			if walked >= ESTIMATE_WALK {
				estimate.exact = false;
				break;
			}
		}
	}

	estimate.delivered = estimate.targeted.saturating_sub(estimate.skipped_opted_out);
	estimate.reach = PLATFORMS
		.iter()
		.filter_map(|platform| reach.get(platform.as_str()))
		.map(|(platform, people)| {
			EstimatedReach {
				platform: *platform,
				people: *people,
			}
		})
		.collect();

	Ok(estimate)
}

/// Adds one page of matched people to the estimate: how many, how many of
/// them opted out, and which platforms the rest can be pushed to.
async fn count_page(
	db: &impl leptoskit_db::Read,
	condition: Option<&crate::condition::Condition>,
	matched: &[Uuid],
	facts: Option<&super::Facts>,
	category: Option<&str>,
	estimate: &mut AudienceEstimate,
	reach: &mut BTreeMap<&'static str, (PushPlatform, u32)>,
) -> rpc::Result<()> {
	if matched.is_empty() {
		return Ok(());
	}

	estimate.targeted = estimate
		.targeted
		.saturating_add(u32::try_from(matched.len()).unwrap_or(u32::MAX));

	let opted_out: BTreeSet<Uuid> = match category {
		Some(category) => {
			Preference::query()
				.filter(|preference| {
					preference.category.eq(category.to_owned())
						& preference.opted_out.eq(true)
						& preference.person.in_list(matched.iter().copied())
				})
				.all(db)
				.await?
				.into_iter()
				.map(|preference| preference.person)
				.collect()
		}
		None => BTreeSet::new(),
	};
	estimate.skipped_opted_out = estimate
		.skipped_opted_out
		.saturating_add(u32::try_from(opted_out.len()).unwrap_or(u32::MAX));

	// A push goes to a live endpoint of someone who gets a row — and, for
	// a condition that asks about devices, only to an endpoint it holds
	// for, as the fanout decides it.
	let delivered = matched
		.iter()
		.copied()
		.filter(|person| !opted_out.contains(person));
	let endpoints = super::push::live_endpoints(db, delivered).await?;
	let device_condition = condition.filter(|condition| condition.mentions_device());

	for (person, endpoints) in &endpoints {
		let mut platforms = BTreeSet::new();

		for endpoint in endpoints {
			let admitted = match (device_condition, facts) {
				(Some(condition), Some(facts)) => {
					facts.holds(condition, *person, Some(super::push::device_of(endpoint)))
				}
				_ => true,
			};

			if admitted && let Some(platform) = PushPlatform::from_str_value(&endpoint.platform) {
				platforms.insert(platform.as_str());
			}
		}

		for name in platforms {
			let platform = PushPlatform::from_str_value(name).unwrap_or(PushPlatform::Apns);
			let entry = reach.entry(platform.as_str()).or_insert((platform, 0));
			entry.1 = entry.1.saturating_add(1);
		}
	}

	Ok(())
}
