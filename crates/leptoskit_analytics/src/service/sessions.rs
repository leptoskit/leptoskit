//! Session rows (§8 of the design): one row per session id, maintained by
//! the batch writer, so web analytics reads sessions rather than every
//! event. Each batch's newly stored events are folded into their
//! session's row: first and last time, duration, entry and exit pages,
//! event and pageview counts, bounce, and how the visit arrived — the
//! channel from the entry pageview's referrer and UTM parameters, and the
//! device from its context.

use std::collections::HashMap;
use std::fmt::Write as _;

use leptoskit_db::Connection as _;
use leptoskit_db::Dialect;
use leptoskit_db::Timestamp;
use leptoskit_db::Tx;
use leptoskit_db::Value;
use serde_json::Value as Json;
use uuid::Uuid;

use super::accept::ContextRow;
use super::accept::Row;
use super::channel::classify;
use super::ingest::Insert;

/// The events that name pages, which a session's entry, exit, referrer,
/// and UTM parameters come from. Other events still count, and still end
/// a bounce.
const PAGE_EVENTS: &[&str] = &["$pageview", "$pageleave", "$screen"];

/// What one batch saw of a session, in arrival order.
struct Visit {
	distinct_id: String,
	person_id: Option<Uuid>,
	first: Timestamp,
	last: Timestamp,
	events: u32,
	pageviews: u32,
	/// The first page-naming event: where the visit came from.
	entry: Option<Page>,
	/// The latest page-naming event: where it was last.
	exit: Option<Page>,
}

/// What a page-naming event says.
#[derive(Clone)]
struct Page {
	timestamp: Timestamp,
	url: String,
	pathname: String,
	referrer_domain: String,
	utm_source: String,
	utm_medium: String,
	utm_campaign: String,
	/// The context the event was sent with, which names the device.
	context_id: i64,
}

/// Folds the batch's rows into their sessions' rows, inside the writer's
/// transaction. `person_ids` are the persons [`super::persons::process`]
/// gave the same rows, and `contexts` maps a context id to its properties.
pub(crate) async fn upsert(
	tx: &Tx,
	dialect: Dialect,
	rows: &[&Row],
	person_ids: &[Option<Uuid>],
	contexts: &HashMap<i64, &ContextRow>,
) -> Result<(), String> {
	let mut visits: Vec<(Uuid, Visit)> = Vec::new();
	for (row, person) in rows.iter().zip(person_ids) {
		let Some(session_id) = row.session_id else {
			continue;
		};
		let when = row.timestamp;
		let page = page_of(row);
		let visit = if let Some((_, visit)) = visits.iter_mut().find(|(id, _)| *id == session_id) {
			visit
		} else {
			visits.push((
				session_id,
				Visit {
					distinct_id: row.distinct_id.clone(),
					person_id: None,
					first: when,
					last: when,
					events: 0,
					pageviews: 0,
					entry: None,
					exit: None,
				},
			));
			&mut visits.last_mut().expect("just pushed").1
		};
		if when < visit.first {
			visit.first = when;
		}
		if when >= visit.last {
			visit.last = when;
			visit.distinct_id.clone_from(&row.distinct_id);
			if person.is_some() {
				visit.person_id = *person;
			}
		} else if visit.person_id.is_none() {
			visit.person_id = *person;
		}
		visit.events += 1;
		if row.name == "$pageview" {
			visit.pageviews += 1;
		}
		if let Some(page) = page {
			match &visit.entry {
				Some(entry) if entry.timestamp <= page.timestamp => {}
				_ => visit.entry = Some(page.clone()),
			}
			match &visit.exit {
				Some(exit) if exit.timestamp > page.timestamp => {}
				_ => visit.exit = Some(page),
			}
		}
	}
	if visits.is_empty() {
		return Ok(());
	}

	// UTM parameters and the referrer belong to the visit's first pageview,
	// later ones to the pages they came from.
	let mut insert = Insert::new(
		dialect,
		"sessions",
		&[
			"id",
			"distinct_id",
			"person_id",
			"first_timestamp",
			"last_timestamp",
			"duration_ms",
			"entry_url",
			"entry_pathname",
			"exit_url",
			"exit_pathname",
			"event_count",
			"pageview_count",
			"is_bounce",
			"channel",
			"referrer_domain",
			"utm_source",
			"utm_medium",
			"utm_campaign",
			"device_type",
			"browser",
			"os",
		],
	);
	for (id, visit) in &mut visits {
		let (entry_url, entry_pathname) = visit
			.entry
			.as_ref()
			.map_or((String::new(), String::new()), |page| {
				(page.url.clone(), page.pathname.clone())
			});
		let (exit_url, exit_pathname) = visit
			.exit
			.as_ref()
			.map_or((String::new(), String::new()), |page| {
				(page.url.clone(), page.pathname.clone())
			});
		let page = visit.entry.as_ref();
		let referrer_domain = page.map_or(String::new(), |page| page.referrer_domain.clone());
		let utm_source = page.map_or(String::new(), |page| page.utm_source.clone());
		let utm_medium = page.map_or(String::new(), |page| page.utm_medium.clone());
		let utm_campaign = page.map_or(String::new(), |page| page.utm_campaign.clone());
		let own = host_of(&entry_url);
		let channel = classify(&utm_source, &utm_medium, &referrer_domain, &own);
		let (device_type, browser, os) = page
			.map_or((String::new(), String::new(), String::new()), |page| {
				device_of(page.context_id, contexts)
			});
		insert.row(vec![
			Value::Uuid(*id),
			Value::Text(visit.distinct_id.clone()),
			visit.person_id.map_or(Value::Null, Value::Uuid),
			Value::Timestamp(visit.first),
			Value::Timestamp(visit.last),
			Value::Int((visit.last - visit.first).num_milliseconds()),
			Value::Text(entry_url),
			Value::Text(entry_pathname),
			Value::Text(exit_url),
			Value::Text(exit_pathname),
			Value::Int(i64::from(visit.events)),
			Value::Int(i64::from(visit.pageviews)),
			Value::Bool(visit.events <= 1 && visit.pageviews <= 1),
			Value::Text(channel.to_owned()),
			Value::Text(referrer_domain),
			Value::Text(utm_source),
			Value::Text(utm_medium),
			Value::Text(utm_campaign),
			Value::Text(device_type),
			Value::Text(browser),
			Value::Text(os),
		]);
	}

	// Fold into what the session's earlier batches wrote: times extend,
	// counts add, and the entry, the first pageview's origin, and the
	// device stay what they were. The duration is the bounds' difference,
	// which each dialect states in its own terms: SQLite keeps timestamps
	// as text, and Postgres subtracts them into an interval.
	let (greatest, least, duration) = match dialect {
		Dialect::Postgres => {
			(
				"GREATEST",
				"LEAST",
				"EXTRACT(EPOCH FROM ({greatest}(\"sessions\".\"last_timestamp\", \
				 excluded.\"last_timestamp\") - {least}(\"sessions\".\"first_timestamp\", \
				 excluded.\"first_timestamp\"))) * 1000",
			)
		}
		Dialect::Sqlite => {
			(
				"MAX",
				"MIN",
				"CAST((julianday({greatest}(\"sessions\".\"last_timestamp\", \
				 excluded.\"last_timestamp\")) - \
				 julianday({least}(\"sessions\".\"first_timestamp\", \
				 excluded.\"first_timestamp\"))) * 86400000 AS INTEGER)",
			)
		}
	};
	let duration = duration
		.replace("{greatest}", greatest)
		.replace("{least}", least);
	let mut statement = insert.finish();
	let _ = write!(
		statement.sql,
		" ON CONFLICT (\"id\") DO UPDATE SET \"first_timestamp\" = \
		 {least}(\"sessions\".\"first_timestamp\", excluded.\"first_timestamp\"), \
		 \"last_timestamp\" = {greatest}(\"sessions\".\"last_timestamp\", \
		 excluded.\"last_timestamp\"), \"duration_ms\" = {duration}, \"entry_url\" = CASE WHEN \
		 excluded.\"first_timestamp\" <= \"sessions\".\"first_timestamp\" THEN \
		 excluded.\"entry_url\" ELSE \"sessions\".\"entry_url\" END, \"entry_pathname\" = CASE \
		 WHEN excluded.\"first_timestamp\" <= \"sessions\".\"first_timestamp\" THEN \
		 excluded.\"entry_pathname\" ELSE \"sessions\".\"entry_pathname\" END, \"exit_url\" = \
		 CASE WHEN excluded.\"last_timestamp\" >= \"sessions\".\"last_timestamp\" THEN \
		 excluded.\"exit_url\" ELSE \"sessions\".\"exit_url\" END, \"exit_pathname\" = CASE WHEN \
		 excluded.\"last_timestamp\" >= \"sessions\".\"last_timestamp\" THEN \
		 excluded.\"exit_pathname\" ELSE \"sessions\".\"exit_pathname\" END, \"event_count\" = \
		 \"sessions\".\"event_count\" + excluded.\"event_count\", \"pageview_count\" = \
		 \"sessions\".\"pageview_count\" + excluded.\"pageview_count\", \"is_bounce\" = \
		 (\"sessions\".\"event_count\" + excluded.\"event_count\") <= 1 AND \
		 (\"sessions\".\"pageview_count\" + excluded.\"pageview_count\") <= 1, \"person_id\" = \
		 COALESCE(excluded.\"person_id\", \"sessions\".\"person_id\"), \"channel\" = CASE WHEN \
		 \"sessions\".\"channel\" = '' THEN excluded.\"channel\" ELSE \"sessions\".\"channel\" \
		 END, \"referrer_domain\" = CASE WHEN \"sessions\".\"referrer_domain\" = '' THEN \
		 excluded.\"referrer_domain\" ELSE \"sessions\".\"referrer_domain\" END, \"utm_source\" = \
		 CASE WHEN \"sessions\".\"utm_source\" = '' THEN excluded.\"utm_source\" ELSE \
		 \"sessions\".\"utm_source\" END, \"utm_medium\" = CASE WHEN \"sessions\".\"utm_medium\" \
		 = '' THEN excluded.\"utm_medium\" ELSE \"sessions\".\"utm_medium\" END, \"utm_campaign\" \
		 = CASE WHEN \"sessions\".\"utm_campaign\" = '' THEN excluded.\"utm_campaign\" ELSE \
		 \"sessions\".\"utm_campaign\" END, \"device_type\" = CASE WHEN \
		 \"sessions\".\"device_type\" = '' THEN excluded.\"device_type\" ELSE \
		 \"sessions\".\"device_type\" END, \"browser\" = CASE WHEN \"sessions\".\"browser\" = '' \
		 THEN excluded.\"browser\" ELSE \"sessions\".\"browser\" END, \"os\" = CASE WHEN \
		 \"sessions\".\"os\" = '' THEN excluded.\"os\" ELSE \"sessions\".\"os\" END"
	);
	tx.execute(statement)
		.await
		.map(|_| ())
		.map_err(|error| error.to_string())
}

/// Reads one row's page properties, when it names a page.
fn page_of(row: &Row) -> Option<Page> {
	if !PAGE_EVENTS.contains(&row.name.as_str()) {
		return None;
	}
	let properties: Json = serde_json::from_str(&row.properties).ok()?;
	let text = |name: &str| {
		properties
			.get(name)
			.and_then(Json::as_str)
			.unwrap_or_default()
			.to_owned()
	};
	Some(Page {
		timestamp: row.timestamp,
		referrer_domain: domain_of(&text("$referrer")),
		url: text("$current_url"),
		pathname: text("$pathname"),
		utm_source: text("utm_source"),
		utm_medium: text("utm_medium"),
		utm_campaign: text("utm_campaign"),
		context_id: row.context_id,
	})
}

/// The session's device, browser, and operating system, from the entry
/// event's context.
fn device_of(context_id: i64, contexts: &HashMap<i64, &ContextRow>) -> (String, String, String) {
	let Some(context) = contexts.get(&context_id) else {
		return (String::new(), String::new(), String::new());
	};
	let Ok(properties) = serde_json::from_str::<Json>(&context.properties) else {
		return (String::new(), String::new(), String::new());
	};
	let text = |name: &str| {
		properties
			.get(name)
			.and_then(Json::as_str)
			.unwrap_or_default()
			.to_owned()
	};
	(text("$device_type"), text("$browser"), text("$os"))
}

/// A URL or a bare domain, as its host: lowercased, without its scheme,
/// path, port, userinfo, or a leading `www.`.
#[must_use]
pub fn domain_of(url: &str) -> String {
	let host = host_of(url);
	host.strip_prefix("www.").unwrap_or(&host).to_owned()
}

/// A URL's host, as [`domain_of`] but keeping `www.`.
#[must_use]
pub fn host_of(url: &str) -> String {
	let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
	let rest = rest.split(['/', '?', '#']).next().unwrap_or(rest);
	let rest = rest.rsplit('@').next().unwrap_or(rest);
	let host = rest.split(':').next().unwrap_or(rest);
	host.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_url_reduces_to_its_host_and_its_domain() {
		assert_eq!(
			host_of("https://App.example:8080/a/b?utm_source=x"),
			"app.example"
		);
		assert_eq!(domain_of("https://www.app.example/a"), "app.example");
		assert_eq!(domain_of("https://app.example/a"), "app.example");
		assert_eq!(domain_of("app.example"), "app.example");
		assert_eq!(domain_of(""), "");
		assert_eq!(
			domain_of("https://user@www.News.example.org:443/x"),
			"news.example.org"
		);
	}

	#[test]
	fn pageviews_and_other_events_together_are_no_bounce_by_count() {
		// The rule the upsert applies, pinned: one pageview and nothing
		// else bounces; any other event doesn't.
		let bounce = |events: u32, pageviews: u32| events <= 1 && pageviews <= 1;
		assert!(bounce(1, 1));
		assert!(!bounce(2, 1));
		assert!(!bounce(1, 2));
		assert!(bounce(1, 0), "a lone non-pageview event is still a bounce");
	}
}
