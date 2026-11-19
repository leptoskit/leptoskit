//! The admin's Analytics area against the real analytics service: the
//! connection signed with the service token, every page drawn from a
//! known October, drilling into the people behind a number, and the
//! guarded actions on a person.

mod support;

use std::fmt::Write as _;

use axum::http::StatusCode;
use axum::http::header;
use leptoskit_analytics::service::JsonEvent;
use serde_json::Value;
use serde_json::json;
use support::*;
use uuid::Uuid;

/// The four weeks the pages read, Monday 5 October to Monday 2 November.
const RANGE: &str = "from=2026-10-05&to=2026-11-02";

/// An instant `day` days after Monday 5 October 2026, at `hour`:`minute`.
fn at(day: u32, hour: u32, minute: u32) -> String {
	let date =
		chrono::NaiveDate::from_ymd_opt(2026, 10, 5).unwrap() + chrono::Days::new(u64::from(day));
	format!("{date}T{hour:02}:{minute:02}:00Z")
}

fn event(name: &str, distinct_id: &str, when: &str, properties: &Value) -> JsonEvent {
	JsonEvent {
		event: name.to_owned(),
		distinct_id: distinct_id.to_owned(),
		uuid: Some(Uuid::now_v7()),
		timestamp: Some(when.to_owned()),
		session_id: None,
		properties: properties.as_object().cloned().unwrap_or_default(),
	}
}

/// A visit: one session's pageviews, a minute apart, from `referrer`.
fn visit(distinct_id: &str, day: u32, hour: u32, pages: &[&str], referrer: &str) -> Vec<JsonEvent> {
	let session = Uuid::now_v7();
	pages
		.iter()
		.zip(0..)
		.map(|(path, minute)| {
			let mut pageview = event(
				"$pageview",
				distinct_id,
				&at(day, hour, minute),
				&json!({
					"$current_url": format!("https://notes.example{path}"),
					"$pathname": path,
					"$referrer": referrer,
					"$browser": "Firefox",
					"$os": "Mac OS X",
					"$device_type": "Desktop",
				}),
			);
			pageview.session_id = Some(session);
			pageview
		})
		.collect()
}

/// October at Notes, small enough to count by hand.
///
/// Ten people visit. Six sign up and are identified as members, so each
/// is one person with two ids. Members come back to write notes in the
/// weeks after; four start a checkout, three submit payment, two finish.
fn october() -> Vec<JsonEvent> {
	let mut events = Vec::new();
	let referrers = [
		"https://www.google.com/",
		"",
		"https://news.ycombinator.com/",
	];

	for visitor in 1..=10_u32 {
		let anon = format!("anon-{visitor:02}");
		let day = (visitor - 1) * 2;
		events.extend(visit(
			&anon,
			day,
			10,
			&["/", "/pricing"],
			referrers[visitor as usize % referrers.len()],
		));
	}

	// Who signs up, on which day, and the weeks they write notes in.
	let members: [(u32, u32, &[u32]); 6] = [
		(1, 0, &[0, 1, 2]),
		(2, 2, &[0, 1]),
		(3, 4, &[0]),
		(4, 8, &[1, 2]),
		(5, 10, &[1]),
		(6, 14, &[2, 3]),
	];

	for (member, day, weeks) in members {
		let anon = format!("anon-{member:02}");
		let email = format!("member-{member:02}@notes.example");
		events.push(event(
			"signed_up",
			&anon,
			&at(day, 10, 5),
			&json!({ "plan": "free", "source": "pricing" }),
		));
		events.push(event(
			"$identify",
			&email,
			&at(day, 10, 6),
			&json!({ "$anon_distinct_id": anon, "$set": { "email": email, "plan": "free" } }),
		));

		for week in weeks {
			// The day after they joined, or the week's Tuesday.
			let note_day = (week * 7 + 1).max(day + 1);
			events.push(event(
				"note_created",
				&email,
				&at(note_day, 15, 0),
				&json!({ "words": 120 + member * 10, "shared": member % 2 == 0 }),
			));
		}
	}

	for (member, steps) in [(1, 3), (2, 2), (4, 3), (5, 1)] {
		let email = format!("member-{member:02}@notes.example");
		let day = 15 + member;
		let names = [
			"checkout_started",
			"payment_submitted",
			"checkout_completed",
		];

		for (minute, name) in (0..).zip(names.iter().take(steps)) {
			events.push(event(
				name,
				&email,
				&at(day, 11, minute * 4),
				&json!({ "plan": "pro", "seats": 2 }),
			));
		}
	}

	events
}

/// The service, seeded with October and its typed events registered, and
/// the admin reading it as the app.
async fn seeded() -> (World, AnalyticsFixture) {
	seeded_with(false).await
}

/// [`seeded`], with the admin told whether its infrastructure is simulated.
async fn seeded_with(simulated: bool) -> (World, AnalyticsFixture) {
	let analytics = analytics_service().await;
	analytics
		.register(&[
			("signed_up", &[("plan", "String"), ("source", "String")]),
			("note_created", &[("words", "u32"), ("shared", "bool")]),
			("checkout_started", &[("plan", "String"), ("seats", "u32")]),
			("payment_submitted", &[("plan", "String"), ("seats", "u32")]),
			(
				"checkout_completed",
				&[("plan", "String"), ("seats", "u32")],
			),
		])
		.await;
	analytics.capture(october()).await;
	let world = world_with_options(Some(analytics.connection(ANALYTICS_TOKEN)), simulated).await;
	(world, analytics)
}

async fn page(world: &World, browser: &mut Browser, path: &str) -> (StatusCode, String) {
	let response = browser.get(&world.admin, ADMIN, path).await;
	let status = response.status();
	(status, text(response).await)
}

/// The value of `attribute` on the element with `test_id`: the first such
/// element, or the last when `last`.
fn attribute_on(html: &str, test_id: &str, attribute: &str, last: bool) -> String {
	let marker = format!("data-testid=\"{test_id}\"");
	let at = if last {
		html.rfind(&marker)
	} else {
		html.find(&marker)
	}
	.unwrap_or_else(|| panic!("no {test_id} on the page"));
	let start = html[..at].rfind('<').unwrap_or(0);
	let end = at + html[at..].find('>').unwrap_or(html.len() - at);
	let tag = &html[start..end];
	let needle = format!(" {attribute}=\"");
	tag.split_once(&needle)
		.map_or_else(
			|| panic!("{test_id} has no {attribute}: {tag}"),
			|(_, value)| value.split('"').next().unwrap_or_default(),
		)
		.replace("&amp;", "&")
		.replace("&quot;", "\"")
}

fn attribute_of(html: &str, test_id: &str, attribute: &str) -> String {
	attribute_on(html, test_id, attribute, false)
}

/// Each Live row's event name, and whether its struct badge shows.
fn live_badges(html: &str) -> Vec<(String, bool)> {
	html.split("data-testid=\"live-event\"")
		.skip(1)
		.map(|row| {
			let name = row.split("aria-description=\"").nth(1).map_or_else(
				|| attribute_of(row, "how-toggle", "aria-label"),
				|value| value[..value.find('"').unwrap()].to_owned(),
			);
			let badge = row
				.find("data-testid=\"struct-badge\"")
				.unwrap_or_else(|| panic!("a row without a badge: {row}"));
			let tag =
				&row[row[..badge].rfind('<').unwrap()..badge + row[badge..].find('>').unwrap()];
			(name, !tag.contains("aria-hidden=\"true\""))
		})
		.collect()
}

/// Replaces what changes between runs: ids, timings, and how long ago.
fn redact(text: &str) -> String {
	// The test service listens on whatever port the system gave it, and the
	// port sits right before the service's path wherever an address is shown,
	// after the colon or, where code is split into tokens, on its own.
	let path = "/_leptoskit/services";
	let text = &text
		.split(path)
		.collect::<Vec<_>>()
		.iter()
		.enumerate()
		.map(|(index, part)| {
			let port = part.len() - part.trim_end_matches(|c: char| c.is_ascii_digit()).len();
			let kept = if port > 0
				&& !part[..part.len() - port].ends_with(|c: char| c.is_alphanumeric())
			{
				format!("{}<port>", &part[..part.len() - port])
			} else {
				(*part).to_owned()
			};

			if index == 0 {
				kept
			} else {
				format!("{path}{kept}")
			}
		})
		.collect::<String>();
	let mut ids: Vec<String> = Vec::new();
	let mut out = String::with_capacity(text.len());
	let chars: Vec<char> = text.chars().collect();
	let mut index = 0;
	let is_uuid = |at: usize| {
		let shape = [8, 4, 4, 4, 12];
		let mut cursor = at;

		for (part, length) in shape.iter().enumerate() {
			for _ in 0..*length {
				if !chars.get(cursor).is_some_and(char::is_ascii_hexdigit) {
					return false;
				}
				cursor += 1;
			}

			if part < 4 {
				if chars.get(cursor) != Some(&'-') {
					return false;
				}
				cursor += 1;
			}
		}

		true
	};

	while index < chars.len() {
		if is_uuid(index) {
			let id: String = chars[index..index + 36].iter().collect();
			let number = ids.iter().position(|seen| *seen == id).unwrap_or_else(|| {
				ids.push(id);
				ids.len() - 1
			});
			let _ = write!(out, "<id {number}>");
			index += 36;
		} else {
			out.push(chars[index]);
			index += 1;
		}
	}

	out.lines()
		.map(|line| {
			let words: Vec<&str> = line.split(' ').collect();
			let mut kept = Vec::with_capacity(words.len());
			let mut index = 0;

			while index < words.len() {
				let bare = |at: usize| {
					words
						.get(at)
						.map(|word| word.trim_matches(|c: char| !c.is_alphanumeric()))
						.unwrap_or_default()
				};
				let number =
					!bare(index).is_empty() && bare(index).chars().all(|c| c.is_ascii_digit());

				if number && bare(index + 2) == "ago" {
					let lead: String = words[index]
						.chars()
						.take_while(|c| !c.is_ascii_digit())
						.collect();
					kept.push(format!("{lead}<n>"));
					kept.push("<unit>".to_owned());
					index += 2;
				} else if number && bare(index + 1) == "ms" {
					let lead: String = words[index]
						.chars()
						.take_while(|c| !c.is_ascii_digit())
						.collect();
					kept.push(format!("{lead}<n>"));
					index += 1;
				} else {
					kept.push(words[index].to_owned());
					index += 1;
				}
			}

			kept.join(" ")
		})
		.collect::<Vec<_>>()
		.join("\n")
}

/// The outline with its actors in name order: the service pages them by
/// person id, and a person's id is random, so their order changes from run
/// to run.
fn by_actor_name(outline: &str) -> String {
	let mut before = Vec::new();
	let mut actors: Vec<Vec<&str>> = Vec::new();
	let mut after = Vec::new();
	let mut indent = None;

	for line in outline.lines() {
		let depth = line.len() - line.trim_start().len();

		if line.trim_start().starts_with("a [actor]") {
			indent = Some(depth);
			actors.push(vec![line]);
		} else if indent.is_some_and(|indent| depth > indent) {
			actors.last_mut().unwrap().push(line);
		} else if actors.is_empty() {
			before.push(line);
		} else {
			indent = None;
			after.push(line);
		}
	}

	actors.sort();
	before
		.into_iter()
		.chain(actors.into_iter().flatten())
		.chain(after)
		.collect::<Vec<_>>()
		.join("\n")
}

fn screen(html: &str) -> String {
	redact(&regions(&outline(html), &["main", "drawer-aside"]))
}

#[tokio::test(flavor = "multi_thread")]
async fn live_workings_measures_a_stored_event() {
	let analytics = analytics_service().await;
	analytics
		.register(&[("note_created", &[("words", "u32"), ("shared", "bool")])])
		.await;
	let now = chrono::Utc::now();
	// The type is known, but its previous event is outside Live's window.
	// Facts are cached independently of the events query.
	analytics
		.capture(vec![event(
			"note_created",
			"earlier-reader",
			&(now - chrono::Duration::days(2)).to_rfc3339(),
			&json!({ "words": 1, "shared": false }),
		)])
		.await;
	let world = world_with(Some(analytics.connection(ANALYTICS_TOKEN))).await;
	let mut ada = world.ada().await;
	let path = format!("{BASE}/analytics/live");
	let (status, html) = page(&world, &mut ada, &path).await;
	assert_eq!(status, StatusCode::OK);
	assert!(
		html.contains("none has arrived yet"),
		"an empty tail has no sample"
	);

	for (name, seconds, properties, explanation) in [
		(
			"$pageview",
			20,
			json!({ "$pathname": "/notes" }),
			"Without a type the names travel with every event",
		),
		(
			"note_created",
			10,
			json!({ "words": 123, "shared": false }),
			"Typed events send no property names",
		),
	] {
		let timestamp = (now - chrono::Duration::seconds(seconds))
			.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
		analytics
			.capture(vec![event(name, "live-reader", &timestamp, &properties)])
			.await;
		let (status, html) = page(&world, &mut ada, &path).await;
		assert_eq!(status, StatusCode::OK);
		assert!(
			html.contains(&format!("the latest {name}, re-encoded here")),
			"Send names the latest {name}"
		);
		assert!(html.contains(explanation), "{name}: {explanation}");
		assert!(
			html.contains("Measured as one event alone in a batch"),
			"{name} explains how its size was measured"
		);
		assert!(
			!html.contains("none has arrived yet"),
			"{name} provides a real sample"
		);
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_service_the_area_teaches_how_to_connect() {
	let world = world().await;
	let mut ada = world.ada().await;

	for path in [
		"overview",
		"trends",
		"funnels",
		"retention",
		"people",
		"live",
		"library",
	] {
		let (status, html) = page(&world, &mut ada, &format!("{BASE}/analytics/{path}")).await;
		assert_eq!(status, StatusCode::OK, "{path}");
		assert!(
			html.contains("data-testid=\"analytics-not-configured\""),
			"{path}"
		);
		assert!(html.contains("LEPTOSKIT_SERVICE_ANALYTICS_TOKEN"), "{path}");
	}

	let response = ada
		.post_json(
			&world.admin,
			ADMIN,
			&format!("{BASE}/api/analytics/insights"),
			json!({ "kind": "trends", "name": "Nothing to save" }),
		)
		.await;
	assert_eq!(response.status(), StatusCode::NOT_FOUND);
	assert_eq!(json_body(response).await["error"], "no_analytics");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_token_is_refused_and_the_page_says_so() {
	let analytics = analytics_service().await;
	let world = world_with(Some(analytics.connection("not-the-token"))).await;
	let mut ada = world.ada().await;

	let (status, html) = page(&world, &mut ada, &format!("{BASE}/analytics/overview")).await;
	assert_eq!(status, StatusCode::OK);
	assert!(
		html.contains("data-testid=\"analytics-unreachable\""),
		"{html}"
	);
	assert!(
		analytics
			.rpc_authorizations()
			.iter()
			.all(|authorization| authorization.as_deref() == Some("Bearer not-the-token")),
		"the admin sends the token it was given"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_page_reads_the_service_as_the_app() {
	let (world, analytics) = seeded().await;
	let mut ada = world.ada().await;
	let snapshot = |name: &str, html: &str| {
		assert!(
			!html.contains("data-testid=\"query-error\""),
			"{name}: {html}"
		);
		insta::assert_snapshot!(name.to_owned(), screen(html));
	};

	let (status, html) = page(
		&world,
		&mut ada,
		&format!("{BASE}/analytics/overview?{RANGE}"),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	// Ten visitors, each with one two-page session; the referrers come out
	// as channels.
	assert!(html.contains(">10<"), "visitors");
	assert!(html.contains(">20<"), "pageviews");
	snapshot("analytics_overview", &html);

	let (_, html) = page(
		&world,
		&mut ada,
		&format!("{BASE}/analytics/trends?{RANGE}&interval=week#workings=trends"),
	)
	.await;
	assert!(html.contains("struct"), "note_created has a Rust type");
	snapshot("analytics_trends", &html);

	let (_, html) = page(
		&world,
		&mut ada,
		&format!("{BASE}/analytics/funnels?{RANGE}"),
	)
	.await;
	assert!(html.contains("50.0%"), "two of four finished checkout");
	snapshot("analytics_funnel", &html);

	let (_, html) = page(
		&world,
		&mut ada,
		&format!("{BASE}/analytics/retention?{RANGE}"),
	)
	.await;
	assert!(html.contains("data-testid=\"retention-cell\""));
	snapshot("analytics_retention", &html);

	let (_, html) = page(
		&world,
		&mut ada,
		&format!("{BASE}/analytics/people?search=member-01"),
	)
	.await;
	let person = attribute_of(&html, "person-row", "href");
	let (_, html) = page(&world, &mut ada, &person).await;
	assert!(html.contains("anon-01") && html.contains("member-01@notes.example"));
	snapshot("analytics_person", &html);

	// Save a trend and put it on a dashboard, as the page's buttons do.
	let saved = ada
		.post_json(
			&world.admin,
			ADMIN,
			&format!("{BASE}/api/analytics/insights"),
			json!({
				"kind": "trends",
				"name": "Notes, weekly",
				"params": { "from": "2026-10-05", "to": "2026-11-02", "interval": "week" },
			}),
		)
		.await;
	assert_eq!(saved.status(), StatusCode::OK);
	let saved = json_body(saved).await;
	let added = ada
		.post_json(
			&world.admin,
			ADMIN,
			&format!("{BASE}/api/analytics/dashboards"),
			json!({
				"kind": "trends",
				"insight": saved["insight"],
				"name": "Notes, weekly",
				"dashboard_name": "Product",
			}),
		)
		.await;
	assert_eq!(added.status(), StatusCode::OK);
	let dashboard = json_body(added).await["href"].as_str().unwrap().to_owned();

	let (_, html) = page(&world, &mut ada, &format!("{BASE}/analytics/library")).await;
	assert!(html.contains("Notes, weekly"));
	let (_, html) = page(&world, &mut ada, &dashboard).await;
	assert_eq!(html.matches("data-testid=\"dashboard-tile\"").count(), 1);
	snapshot("analytics_dashboard", &html);

	// Learn reads the same facts: a typed event arrived, a trend was saved,
	// and a person was merged from two ids.
	let (_, html) = page(&world, &mut ada, &format!("{BASE}/learn")).await;
	assert!(html.contains("Analytics 3 of 6"), "{html}");
	// Every member signed up in the same batch, so which of them is the
	// newest merge is a tie; any one of them is right.
	let merged = "@notes.example's anonymous visits and account are one person";
	let member = html.find(merged).expect("a merged person");
	assert!(
		html[..member].ends_with(|c: char| c.is_ascii_digit()),
		"{html}"
	);
	let html = format!(
		"{}member-0N{}",
		&html[..member - "member-0N".len()],
		&html[member..]
	);
	snapshot("analytics_learn", &html);

	// The live page starts from the latest events and follows the tail.
	// Every row carries the struct badge, shown only for a typed event, so
	// the script can set it from each new event whichever row it copies.
	let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
	analytics
		.capture(vec![
			event(
				"note_created",
				"anon-live",
				&now,
				&json!({ "words": 3, "shared": false }),
			),
			event(
				"$pageview",
				"anon-live",
				&now,
				&json!({ "$pathname": "/notes" }),
			),
		])
		.await;
	let (_, html) = page(&world, &mut ada, &format!("{BASE}/analytics/live")).await;
	assert!(html.contains("data-testid=\"live-events\""));
	assert!(html.contains("data-testid=\"live-pause\""));
	let rows = live_badges(&html);
	assert!(
		rows.contains(&("note_created".to_owned(), true))
			&& rows.contains(&("$pageview".to_owned(), false)),
		"{rows:?}"
	);
	assert!(
		rows.iter()
			.all(|(name, shown)| *shown == (name == "note_created" || name == "signed_up")),
		"{rows:?}"
	);

	// Every call the admin made carried the service's token.
	let authorizations = analytics.rpc_authorizations();
	assert!(authorizations.len() > 10, "{authorizations:?}");
	let expected = format!("Bearer {ANALYTICS_TOKEN}");
	assert!(
		authorizations
			.iter()
			.all(|authorization| authorization.as_deref() == Some(expected.as_str())),
		"{authorizations:?}"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_number_opens_onto_the_people_behind_it() {
	let (world, _analytics) = seeded().await;
	let mut ada = world.ada().await;
	let names = |html: &str| {
		let mut found: Vec<String> = (1..=10)
			.map(|member| format!("member-{member:02}@notes.example"))
			.filter(|email| html.contains(email.as_str()))
			.collect();
		found.sort();
		found
	};

	// The funnel's drop-off after the first step: Member 5 started a
	// checkout and never paid.
	let (_, html) = page(
		&world,
		&mut ada,
		&format!("{BASE}/analytics/funnels?{RANGE}"),
	)
	.await;
	let drop_off = attribute_of(&html, "funnel-drop-off", "href");
	let (status, html) = page(&world, &mut ada, &drop_off).await;
	assert_eq!(status, StatusCode::OK);
	assert!(html.contains("data-testid=\"actors\""), "{html}");
	assert_eq!(names(&html), ["member-05@notes.example"]);

	// Everyone who reached the last step.
	let (_, html) = page(
		&world,
		&mut ada,
		&format!("{BASE}/analytics/funnels?{RANGE}"),
	)
	.await;
	let step = attribute_on(&html, "funnel-step", "href", true);
	let (_, html) = page(&world, &mut ada, &step).await;
	assert_eq!(
		names(&html),
		["member-01@notes.example", "member-04@notes.example"]
	);

	// A retention cell: the first week's cohort, back the same week.
	let (_, html) = page(
		&world,
		&mut ada,
		&format!("{BASE}/analytics/retention?{RANGE}"),
	)
	.await;
	let cells: Vec<&str> = html.split("data-testid=\"retention-cell\"").collect();
	assert!(cells.len() > 2, "{html}");
	let (_, html) = page(
		&world,
		&mut ada,
		&attribute_of(&html, "retention-cell", "href"),
	)
	.await;
	assert!(html.contains("data-testid=\"actor\""), "{html}");
	insta::assert_snapshot!(
		"analytics_actors",
		redact(&by_actor_name(&regions(
			&outline(&html),
			&["main", "drawer-aside"]
		)))
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn export_and_delete_need_a_confirmation_and_the_role() {
	let (world, _analytics) = seeded().await;
	let mut ada = world.ada().await;
	let (_, html) = page(
		&world,
		&mut ada,
		&format!("{BASE}/analytics/people?search=member-03"),
	)
	.await;
	let person = attribute_of(&html, "person-row", "href");
	let id = person.rsplit('/').next().unwrap().to_owned();
	let (_, html) = page(&world, &mut ada, &person).await;
	let guard = attribute_of(&html, "person-delete", "aria-description");
	let (_, confirmation) = guard.split_once('|').unwrap();
	assert_eq!(confirmation, "member-03@notes.example");

	// Export: everything stored about them, as a download.
	let response = ada
		.get(
			&world.admin,
			ADMIN,
			&format!("{BASE}/api/analytics/people/{id}/export"),
		)
		.await;
	assert_eq!(response.status(), StatusCode::OK);
	assert!(
		response.headers()[header::CONTENT_DISPOSITION]
			.to_str()
			.unwrap()
			.starts_with("attachment")
	);
	let export = text(response).await;
	assert!(
		export.contains("anon-03") && export.contains("note_created"),
		"{export}"
	);

	let delete = |confirm: &str| json!({ "confirm": confirm });
	let path = format!("{BASE}/api/analytics/people/{id}/delete");

	// The wrong name deletes nothing.
	let response = ada
		.post_json(&world.admin, ADMIN, &path, delete("member-03"))
		.await;
	assert_eq!(response.status(), StatusCode::BAD_REQUEST);
	assert_eq!(json_body(response).await["error"], "confirmation");
	assert_eq!(page(&world, &mut ada, &person).await.0, StatusCode::OK);

	// Someone without a session can't, and nor can Ada once the main site
	// takes her role away.
	let mut stranger = Browser::default();
	let response = stranger
		.post_json(&world.admin, ADMIN, &path, delete(confirmation))
		.await;
	assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
	world
		.admin_db
		.run_script("UPDATE admin_roles SET roles = ''")
		.await
		.unwrap();
	let response = ada
		.post_json(&world.admin, ADMIN, &path, delete(confirmation))
		.await;
	assert_eq!(response.status(), StatusCode::FORBIDDEN);
	assert_eq!(json_body(response).await["error"], "not_admin");
	let response = ada
		.get(
			&world.admin,
			ADMIN,
			&format!("{BASE}/api/analytics/people/{id}/export"),
		)
		.await;
	assert_eq!(response.status(), StatusCode::FORBIDDEN);
	world
		.admin_db
		.run_script("UPDATE admin_roles SET roles = 'admin'")
		.await
		.unwrap();

	// The name typed out deletes them, ids and all.
	let response = ada
		.post_json(&world.admin, ADMIN, &path, delete(confirmation))
		.await;
	assert_eq!(response.status(), StatusCode::OK);
	assert_eq!(
		page(&world, &mut ada, &person).await.0,
		StatusCode::NOT_FOUND
	);
	let (_, html) = page(
		&world,
		&mut ada,
		&format!("{BASE}/analytics/people?search=member-03"),
	)
	.await;
	assert!(!html.contains("data-testid=\"person-row\""));
}

/// Writes real pages, as the admin renders them, into the directory
/// `LEPTOSKIT_ADMIN_PAGES` names, for `tests/script.mjs` to drive the
/// admin's script against in a browser. It runs from there.
#[tokio::test]
#[ignore = "tests/script.mjs runs it"]
async fn pages_for_the_script() {
	let directory = std::path::PathBuf::from(
		std::env::var_os("LEPTOSKIT_ADMIN_PAGES").expect("set LEPTOSKIT_ADMIN_PAGES"),
	);
	let (world, analytics) = seeded_with(true).await;
	let mut ada = world.ada().await;

	// The tail's newest event is untyped, so the next typed one copies a
	// row without a visible badge.
	let now = chrono::Utc::now();
	let at = |seconds: i64| {
		(now - chrono::Duration::seconds(seconds))
			.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
	};
	analytics
		.capture(vec![
			event(
				"note_created",
				"anon-live",
				&at(20),
				&json!({ "words": 3, "shared": false }),
			),
			event(
				"$pageview",
				"anon-live",
				&at(10),
				&json!({ "$pathname": "/notes" }),
			),
		])
		.await;

	// A plan with changes, for Changes.
	let response = ada
		.post_json(
			&world.admin,
			ADMIN,
			&format!("{BASE}/api/projects/notes-production/actions"),
			json!({ "kind": "plan", "idempotency_key": "plan" }),
		)
		.await;
	assert!(response.status().is_success(), "{response:?}");
	let claimed = world
		.store
		.claim_project(Some("notes-production"))
		.unwrap()
		.unwrap();
	world
		.store
		.finish(
			&claimed.id,
			&leptoskit_deploy_console::control::RunResult {
				succeeded: true,
				message: "Completed. The plan was compared with the simulated cloud.".into(),
				details: json!({
					"code": "completed",
					"changes": [{ "address": "hcloud_server.web[1]", "actions": ["create"] }],
				}),
			},
		)
		.unwrap();

	let project = format!("{BASE}/infra/projects/notes-production");
	for (file, path) in [
		(
			"trends-day.html",
			format!("{BASE}/analytics/trends?{RANGE}&interval=day"),
		),
		(
			"trends-week.html",
			format!("{BASE}/analytics/trends?{RANGE}&interval=week"),
		),
		(
			"trends-pageviews.html",
			format!("{BASE}/analytics/trends?{RANGE}&s0=%24pageview&interval=day"),
		),
		(
			"trends-channels.html",
			format!(
				"{BASE}/analytics/trends?{RANGE}&s0=%24pageview&interval=day&by=session:channel"
			),
		),
		("live.html", format!("{BASE}/analytics/live")),
		("glossary.html", format!("{BASE}/learn/glossary")),
		("learn.html", format!("{BASE}/learn")),
		("overview.html", format!("{project}/overview")),
		(
			"overview-release.html",
			format!("{project}/overview?workings=release"),
		),
		("releases.html", format!("{project}/releases")),
		("changes.html", format!("{project}/changes")),
		("projects.html", format!("{BASE}/infra/projects")),
		(
			"operation.html",
			format!("{BASE}/infra/operations/{}", claimed.id),
		),
		(
			"operation-workings.html",
			format!("{BASE}/infra/operations/{}?workings=operation", claimed.id),
		),
	] {
		let (status, html) = page(&world, &mut ada, &path).await;
		assert_eq!(status, StatusCode::OK, "{path}");
		std::fs::write(directory.join(file), html).unwrap();
	}
}
