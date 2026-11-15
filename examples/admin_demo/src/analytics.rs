//! The demo's analytics: the real `leptoskit_analytics` service, its
//! typed events, and a month of traffic to seed it with.
//!
//! [`start`] runs the service on its own SQLite database, as `lk dev` would
//! run it beside an app; the caller passes the token its callers present to
//! count as the app, and nothing outside the process that serves the
//! routes should accept it. [`seed`] sends the month over HTTP; [`seed_with`]
//! drives the service's own routes instead, so a caller can seed its
//! generation before it serves. Either way the visits go through the same
//! ingestion path a client uses (`POST v1/batch`, signed as the app):
//! sessions from six channels on three kinds of device, sign-ups that
//! identify their anonymous visitor, a checkout funnel that loses people
//! at each step, and people who come back, less each week.

use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use axum::Router;
use axum::http::Method;
use axum::http::Request;
use axum::http::header;
use leptoskit_analytics::Event;
use leptoskit_analytics::service;
use leptoskit_analytics::service::CaptureResult;
use leptoskit_analytics::service::catalogue::SchemaInput;
use leptoskit_analytics::wire;
use leptoskit_analytics::wire::Body;
use leptoskit_analytics::wire::Value;
use leptoskit_db::Db;
use leptoskit_rpc::client::AppCaller;
use leptoskit_rpc::server::APP_CALLER;
use leptoskit_rpc::server::Anonymous;
use leptoskit_rpc::server::ServiceCallers;
use tower::ServiceExt as _;

/// How many bytes of a refused response an error message quotes.
const REFUSED_BYTES: usize = 16 * 1024;

/// What seeding through the service's own routes came to, from the
/// service's typed answers: bounded, checkable evidence, not a guess
/// from what was sent.
///
/// `entries` counts every wire entry; `events` and `identities` split
/// them by what they are — named and typed events versus the
/// identity/property entries (`$identify`, `$create_alias`, `$set`)
/// that are entries of their own. Every *accepted* entry becomes an
/// event row, which is why a read-back of the catalogue counts both;
/// the split is what keeps the receipt honest about which is which.
/// `stored` and `duplicates` are the service's own numbers, summed per
/// batch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeedReceipt {
	/// Wire entries sent.
	pub entries: u64,
	/// Entries that were events: `Event` and `TypedEvent` bodies.
	pub events: u64,
	/// Entries that were identity/property changes.
	pub identities: u64,
	/// Rows the service reported stored.
	pub stored: u64,
	/// Rows the service reported as already stored by an earlier attempt.
	pub duplicates: u64,
}

/// Someone signed up.
#[derive(Event)]
pub struct SignedUp {
	pub plan: String,
	pub source: String,
}

/// Someone wrote a note: what the app is for.
#[derive(Event)]
pub struct NoteCreated {
	pub words: u32,
	pub shared: bool,
}

/// The checkout funnel's three steps.
#[derive(Event)]
pub struct CheckoutStarted {
	pub plan: String,
	pub seats: u32,
}

#[derive(Event)]
pub struct PaymentSubmitted {
	pub plan: String,
	pub method: String,
}

#[derive(Event)]
pub struct CheckoutCompleted {
	pub plan: String,
	pub seats: u32,
	pub amount_cents: u32,
}

fn schema<E: Event>() -> SchemaInput {
	SchemaInput {
		schema_id: E::SCHEMA_ID,
		name: E::NAME.to_owned(),
		fields: E::FIELDS
			.iter()
			.map(|(name, ty)| ((*name).to_owned(), (*ty).to_owned()))
			.collect(),
	}
}

/// Every typed event the site sends.
pub fn schemas() -> Vec<SchemaInput> {
	vec![
		schema::<SignedUp>(),
		schema::<NoteCreated>(),
		schema::<CheckoutStarted>(),
		schema::<PaymentSubmitted>(),
		schema::<CheckoutCompleted>(),
	]
}

/// The service's routes, its database migrated, its live stream started,
/// its files under `data`. `token` is the service's token for the app's
/// own callers, which the caller keeps: whoever presents it registers and
/// reads as the app. Nothing outside the process serving these routes
/// accepts it.
pub async fn start(data: &Path, origin: &str, token: &str) -> Result<Router> {
	let db = Db::connect(&format!(
		"sqlite://{}",
		data.join("analytics.sqlite3").display()
	))
	.await?;
	db.migrate(service::MIGRATIONS).await?;
	let (router, live) = service::routes_with(
		db,
		Some(origin),
		service::IngestConfig::default(),
		ServiceCallers::new(Anonymous).caller(token, APP_CALLER),
	);
	live.start().await?;
	Ok(router)
}

/// The app's typed events, put in the service at `url`'s catalogue, as the
/// app's server does when it starts, signed with `token`.
pub async fn register(url: &str, token: &str) -> Result<()> {
	AppCaller::new(leptoskit_analytics::SERVICE, url, token)
		.call(service::RegisterEvents { schemas: schemas() })
		.await
		.map_err(|error| anyhow::anyhow!("registering the typed events: {}", error.message()))?;
	Ok(())
}

/// A small, seeded random source, so every seed makes the same month.
struct Dice(u64);

impl Dice {
	fn next(&mut self) -> u64 {
		self.0 ^= self.0 << 13;
		self.0 ^= self.0 >> 7;
		self.0 ^= self.0 << 17;
		self.0
	}

	/// True with probability `p`.
	#[expect(clippy::cast_precision_loss, reason = "a probability")]
	fn chance(&mut self, p: f64) -> bool {
		(self.next() % 10_000) as f64 / 10_000.0 < p
	}

	fn below(&mut self, n: u64) -> u64 {
		self.next() % n.max(1)
	}

	fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
		&items[usize::try_from(self.below(items.len() as u64)).unwrap_or(0)]
	}

	fn bytes(&mut self) -> [u8; 10] {
		let mut bytes = [0; 10];
		bytes[..8].copy_from_slice(&self.next().to_le_bytes());
		bytes[8..].copy_from_slice(&self.next().to_le_bytes()[..2]);
		bytes
	}
}

/// Where a visit came from: referrer, then `utm_source`, `utm_medium`,
/// `utm_campaign`, and how often.
const ORIGINS: &[(&str, &str, &str, &str, u64)] = &[
	("", "", "", "", 30),
	("https://www.google.com/", "", "", "", 26),
	("https://news.ycombinator.com/", "", "", "", 12),
	("https://t.co/", "twitter", "social", "launch", 10),
	("", "newsletter", "email", "november", 10),
	(
		"https://www.google.com/",
		"google",
		"cpc",
		"notes-search",
		7,
	),
	("https://www.reddit.com/", "", "", "", 5),
];

/// Device, browser, operating system, and how often.
const DEVICES: &[(&str, &str, &str, u64)] = &[
	("Desktop", "Chrome", "Mac OS X", 30),
	("Desktop", "Safari", "Mac OS X", 14),
	("Desktop", "Chrome", "Windows", 18),
	("Desktop", "Firefox", "Linux", 6),
	("Mobile", "Mobile Safari", "iOS", 20),
	("Mobile", "Chrome", "Android", 10),
	("Tablet", "Mobile Safari", "iOS", 2),
];

const PAGES: &[&str] = &[
	"/",
	"/features",
	"/pricing",
	"/blog/typed-events",
	"/docs",
	"/changelog",
];

fn weighted<'a, T>(dice: &mut Dice, items: &'a [T], weight: impl Fn(&T) -> u64) -> &'a T {
	let total: u64 = items.iter().map(&weight).sum();
	let mut roll = dice.below(total);

	for item in items {
		let weight = weight(item);
		if roll < weight {
			return item;
		}
		roll -= weight;
	}

	&items[0]
}

fn text(value: &str) -> Value {
	Value::Text(value.to_owned())
}

/// Builds batches as the month happens.
struct Month {
	batch: wire::Batch,
	dice: Dice,
}

impl Month {
	fn context(&mut self, device: &(&str, &str, &str, u64)) -> u32 {
		let properties = vec![
			("$lib".to_owned(), text("leptoskit-web")),
			("$device_type".to_owned(), text(device.0)),
			("$browser".to_owned(), text(device.1)),
			("$os".to_owned(), text(device.2)),
		];

		if let Some(index) = self
			.batch
			.contexts
			.iter()
			.position(|known| *known == properties)
		{
			return u32::try_from(index).unwrap_or(0);
		}

		self.batch.contexts.push(properties);
		u32::try_from(self.batch.contexts.len() - 1).unwrap_or(0)
	}

	fn entry(
		&mut self,
		millis: i64,
		distinct_id: &str,
		session: Option<[u8; 16]>,
		context: u32,
		body: Body,
	) {
		let random = self.dice.bytes();
		self.batch.entries.push(wire::Entry {
			uuid: wire::uuid_v7(millis, random),
			distinct_id: distinct_id.to_owned(),
			session_id: session,
			context,
			body,
		});
	}

	fn typed<E: Event>(
		&mut self,
		millis: i64,
		distinct_id: &str,
		session: [u8; 16],
		context: u32,
		event: E,
	) {
		self.entry(
			millis,
			distinct_id,
			Some(session),
			context,
			Body::TypedEvent {
				name: E::NAME.to_owned(),
				schema_id: E::SCHEMA_ID,
				values: event.into_values(),
				properties: Vec::new(),
			},
		);
	}

	/// One session: pageviews, and whatever the visitor does there.
	fn session(
		&mut self,
		start: i64,
		distinct_id: &str,
		origin: &(&str, &str, &str, &str, u64),
		device: &(&str, &str, &str, u64),
		first_page: &str,
		pages: u64,
	) -> ([u8; 16], u32, i64) {
		let session = wire::uuid_v7(start, self.dice.bytes());
		let context = self.context(device);
		let mut at = start;

		for index in 0..pages {
			let path = if index == 0 {
				first_page
			} else {
				*self.dice.pick(PAGES)
			};
			let mut properties = vec![
				(
					"$current_url".to_owned(),
					text(&format!("https://notes.example{path}")),
				),
				("$pathname".to_owned(), text(path)),
			];

			if index == 0 {
				properties.push(("$referrer".to_owned(), text(origin.0)));
				for (key, value) in [
					("utm_source", origin.1),
					("utm_medium", origin.2),
					("utm_campaign", origin.3),
				] {
					if !value.is_empty() {
						properties.push((key.to_owned(), text(value)));
					}
				}
			}

			self.entry(
				at,
				distinct_id,
				Some(session),
				context,
				Body::Event {
					name: "$pageview".to_owned(),
					properties,
				},
			);
			#[expect(clippy::cast_possible_wrap, reason = "seconds between pages")]
			let pause = (8 + self.dice.below(70)) as i64 * 1000;
			at += pause;
		}

		(session, context, at)
	}
}

/// A month of visits, ending now: today and the 29 days before it, the same
/// days the admin's "Last 30 days" covers, so the previous period it
/// compares against is empty rather than holding a sliver of the seed.
#[expect(clippy::too_many_lines, reason = "one month, day by day")]
fn month(now_millis: i64) -> Vec<wire::Batch> {
	const DAY: i64 = 86_400_000;
	let start = now_millis - now_millis.rem_euclid(DAY) - 29 * DAY;
	let mut month = Month {
		batch: wire::Batch::default(),
		dice: Dice(0x5eed_1e47_0b5e_55ed),
	};
	let mut batches = Vec::new();
	// People who signed up: their id, sign-up time, device, and plan.
	let mut members: Vec<(String, i64, usize, &str)> = Vec::new();
	let mut visitor = 0_u32;

	for day in 0..30_i64 {
		// Traffic grows through the month, with quieter weekends.
		let weekday = (day + 3) % 7;
		let base = 46 + day * 2;
		let visitors = if weekday >= 5 { base * 2 / 3 } else { base };

		for _ in 0..visitors {
			visitor += 1;
			let anonymous = format!("anon-{visitor:05}");
			let origin = *weighted(&mut month.dice, ORIGINS, |origin| origin.4);
			let device_index = {
				let device = weighted(&mut month.dice, DEVICES, |device| device.3);
				DEVICES
					.iter()
					.position(|known| std::ptr::eq(known, device))
					.unwrap_or(0)
			};
			let device = DEVICES[device_index];
			#[expect(clippy::cast_possible_wrap, reason = "a time within the day")]
			let at = start + day * DAY + (7 * 3_600_000) + month.dice.below(14 * 3_600_000) as i64;

			if at >= now_millis - 120_000 {
				continue;
			}

			let landing = *weighted(
				&mut month.dice,
				&[
					("/", 50),
					("/pricing", 15),
					("/blog/typed-events", 20),
					("/features", 15),
				],
				|page| page.1,
			);
			let bounced = month.dice.chance(0.38);
			let pages = if bounced { 1 } else { 2 + month.dice.below(4) };
			let (session, context, end) =
				month.session(at, &anonymous, &origin, &device, landing.0, pages);

			// About a third of those who stay sign up, then identify.
			if !bounced && month.dice.chance(0.34) {
				let email = format!("member{visitor:05}@notes.example");
				let plan = *month.dice.pick(&["free", "free", "pro", "team"]);
				month.typed(
					end,
					&anonymous,
					session,
					context,
					SignedUp {
						plan: plan.to_owned(),
						source: if origin.1.is_empty() {
							"organic".to_owned()
						} else {
							origin.1.to_owned()
						},
					},
				);
				month.entry(
					end + 500,
					&email,
					Some(session),
					context,
					Body::Identify {
						anon_distinct_id: anonymous.clone(),
						set: vec![
							("email".to_owned(), text(&email)),
							("name".to_owned(), text(&format!("Member {visitor}"))),
							("plan".to_owned(), text(plan)),
						],
						set_once: vec![(
							"first_channel".to_owned(),
							text(if origin.0.is_empty() {
								"direct"
							} else {
								origin.0
							}),
						)],
					},
				);
				let note = NoteCreated {
					words: u32::try_from(20 + month.dice.below(400)).unwrap_or(20),
					shared: month.dice.chance(0.2),
				};
				month.typed(end + 30_000, &email, session, context, note);
				members.push((email, end, device_index, plan));
			}
		}

		// Members come back, less each week after they join. The odds are
		// per day, so a week's comes out near 45%, 32%, 27%, then 24%.
		let today = start + day * DAY;
		let returning: Vec<(String, i64, usize, &str)> = members
			.iter()
			.filter(|(_, joined, ..)| *joined < today)
			.cloned()
			.collect();

		for (email, joined, device_index, plan) in returning {
			let weeks = (today - joined) / (7 * DAY);
			let p = match weeks {
				0 => 0.082,
				1 => 0.054,
				2 => 0.044,
				_ => 0.038,
			};

			if !month.dice.chance(p) {
				continue;
			}

			#[expect(clippy::cast_possible_wrap, reason = "a time within the day")]
			let at = today + 8 * 3_600_000 + month.dice.below(12 * 3_600_000) as i64;

			if at >= now_millis - 120_000 {
				continue;
			}

			let device = DEVICES[device_index];
			let pages = 1 + month.dice.below(3);
			let (session, context, mut end) =
				month.session(at, &email, &ORIGINS[0], &device, "/notes", pages);

			for _ in 0..=month.dice.below(3) {
				let note = NoteCreated {
					words: u32::try_from(20 + month.dice.below(600)).unwrap_or(20),
					shared: month.dice.chance(0.25),
				};
				month.typed(end, &email, session, context, note);
				end += 45_000;
			}

			// Free members sometimes try to upgrade, and some get through.
			if plan == "free" && month.dice.chance(0.16) {
				let seats = 1 + u32::try_from(month.dice.below(4)).unwrap_or(0);
				let target = if seats > 1 { "team" } else { "pro" };
				month.typed(
					end,
					&email,
					session,
					context,
					CheckoutStarted {
						plan: target.to_owned(),
						seats,
					},
				);

				if month.dice.chance(0.64) {
					let paid_at =
						end + 60_000 + i64::try_from(month.dice.below(240_000)).unwrap_or(0);
					let payment = PaymentSubmitted {
						plan: target.to_owned(),
						method: (*month.dice.pick(&["card", "card", "card", "apple_pay"]))
							.to_owned(),
					};
					month.typed(paid_at, &email, session, context, payment);

					let price = if target == "team" { 1_200 } else { 800 };

					if month.dice.chance(0.81) {
						month.typed(
							end + 400_000,
							&email,
							session,
							context,
							CheckoutCompleted {
								plan: target.to_owned(),
								seats,
								amount_cents: seats * price,
							},
						);
					}
				}
			}
		}

		// A day per batch keeps each under the service's limits.
		month.batch.sent_at = now_millis;
		batches.push(std::mem::take(&mut month.batch));
	}

	batches
}

/// Sends a month of traffic to the service at `url`, as the app signed
/// with `token`.
pub async fn seed(url: &str, token: &str) -> Result<usize> {
	register(url, token).await?;
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)?
		.as_millis();
	let batches = month(i64::try_from(now)?);
	let client = reqwest::Client::new();
	let mut sent = 0;

	for batch in &batches {
		let response = client
			.post(format!(
				"{url}{}/v1/batch",
				leptoskit_rpc::service_prefix(leptoskit_analytics::SERVICE)
			))
			.bearer_auth(token)
			.header(header::CONTENT_TYPE, "application/octet-stream")
			.body(wire::encode(batch))
			.send()
			.await
			.context("sending a seed batch")?;
		anyhow::ensure!(
			response.status().is_success(),
			"the service refused a seed batch ({}): {}",
			response.status(),
			response.text().await.unwrap_or_default()
		);
		sent += batch.entries.len();
	}

	Ok(sent)
}

/// Registers the typed events and sends a month of traffic through
/// `router`, the service's own routes, as the app signed with `token`:
/// what [`seed`] does over HTTP, without a network. The month ends at
/// `now_millis`, so a caller that seeds before it serves passes the time
/// it started at, and the first reader sees a month that's current.
///
/// Every batch's answer is checked as it comes back — [`SeedReceipt`] is
/// evidence from the service, not from the sender — so a partially
/// refused, rate-limited, or unaccounted seed is an error rather than a
/// month that silently publishes incomplete.
///
/// # Errors
///
/// When registration or a batch is refused, when the service's answer
/// cannot be read, or when the answer does not account for exactly the
/// entries sent.
pub async fn seed_with(router: Router, token: &str, now_millis: i64) -> Result<SeedReceipt> {
	register_with(&router, token).await?;
	seed_batches(&router, token, &month(now_millis)).await
}

/// Sends the batches through the service's own routes, checking every
/// answer. Private: the seam a test uses to prove what a poisoned batch
/// comes to, against the real router.
async fn seed_batches(
	router: &Router,
	token: &str,
	batches: &[wire::Batch],
) -> Result<SeedReceipt> {
	let mut receipt = SeedReceipt::default();

	for batch in batches {
		let events = batch
			.entries
			.iter()
			.filter(|entry| is_event(&entry.body))
			.count() as u64;
		let identities = batch.entries.len() as u64 - events;

		let response = router
			.clone()
			.oneshot(
				Request::builder()
					.method(Method::POST)
					.uri(format!(
						"{}/v1/batch",
						leptoskit_rpc::service_prefix(leptoskit_analytics::SERVICE)
					))
					.header(header::AUTHORIZATION, format!("Bearer {token}"))
					.header(header::CONTENT_TYPE, "application/octet-stream")
					.body(axum::body::Body::from(wire::encode(batch)))?,
			)
			.await
			.context("sending a seed batch")?;
		let status = response.status();
		let answer = axum::body::to_bytes(response.into_body(), REFUSED_BYTES)
			.await
			.context("reading the seed batch's answer")?;
		ensure!(
			status.is_success(),
			"the service refused a seed batch ({status}): {}",
			String::from_utf8_lossy(&answer)
		);
		let answer: CaptureResult = serde_json::from_slice(&answer).with_context(|| {
			format!(
				"reading the answer as a capture result: {}",
				String::from_utf8_lossy(&answer)
			)
		})?;

		// A seed that publishes must be complete: refusals are permanent,
		// rate-limited entries were dropped, and an answer that does not
		// account for every entry sent leaves rows unexplained.
		ensure!(
			answer.rate_limited == 0,
			"the service rate-limited {} of the seed's entries: the month publishes only whole",
			answer.rate_limited
		);
		ensure!(
			answer.refused.is_empty(),
			"the service refused {} of the seed's entries (first: {:?}): the month publishes only \
			 whole",
			answer.refused.len(),
			answer.refused.first()
		);
		let accounted = u64::from(answer.stored)
			+ u64::from(answer.duplicates)
			+ u64::from(answer.rate_limited)
			+ answer.refused.len() as u64;
		let sent = batch.entries.len() as u64;
		ensure!(
			accounted == sent,
			"the service's answer accounts for {accounted} of the {sent} entries sent: the month \
			 publishes only a fully accounted seed"
		);

		receipt.entries += sent;
		receipt.events += events;
		receipt.identities += identities;
		receipt.stored += u64::from(answer.stored);
		receipt.duplicates += u64::from(answer.duplicates);
	}

	Ok(receipt)
}

/// Whether a wire entry is an event — named or typed — rather than one
/// of the identity/property changes that are entries of their own. The
/// distinction is the wire's, and the receipt keeps it: an answer's
/// numbers cover every entry, while a marker's seed evidence is about
/// events first.
const fn is_event(entry_body: &Body) -> bool {
	matches!(entry_body, Body::Event { .. } | Body::TypedEvent { .. })
}

/// Puts the typed events in `router`'s catalogue, as the app signed with
/// `token`, through the service's own `register_events` route.
async fn register_with(router: &Router, token: &str) -> Result<()> {
	let arguments = service::RegisterEvents { schemas: schemas() };
	let response = router
		.clone()
		.oneshot(
			Request::builder()
				.method(Method::POST)
				.uri(format!(
					"{}/rpc/register_events",
					leptoskit_rpc::service_prefix(leptoskit_analytics::SERVICE)
				))
				.header(header::ACCEPT, "application/json")
				.header(header::CONTENT_TYPE, "application/json")
				.header(header::AUTHORIZATION, format!("Bearer {token}"))
				.body(axum::body::Body::from(serde_json::to_vec(&arguments)?))?,
		)
		.await
		.context("registering the typed events")?;
	let status = response.status();
	anyhow::ensure!(
		status.is_success(),
		"the service refused the typed events ({status}): {}",
		quote_refusal(response).await
	);
	Ok(())
}

/// A refused response's body, for the error message.
async fn quote_refusal(response: axum::response::Response) -> String {
	let Ok(bytes) = axum::body::to_bytes(response.into_body(), REFUSED_BYTES).await else {
		return String::new();
	};
	String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use axum::http::StatusCode;

	use super::*;

	/// A directory only the tests write, wiped between runs. Each test
	/// names its own: they run in parallel.
	fn scratch(name: &str) -> PathBuf {
		let data = std::env::temp_dir().join(format!(
			"admin-demo-seed-test-{name}-{}",
			std::process::id()
		));
		let _ = std::fs::remove_dir_all(&data);
		data
	}

	/// Asks the service's `path`, as the app signed with `token`.
	async fn ask(router: &Router, path: &str, token: &str) -> axum::response::Response {
		router
			.clone()
			.oneshot(
				Request::builder()
					.method(Method::POST)
					.uri(path)
					.header(header::ACCEPT, "application/json")
					.header(header::CONTENT_TYPE, "application/json")
					.header(header::AUTHORIZATION, format!("Bearer {token}"))
					.body(axum::body::Body::empty())
					.expect("the request builds"),
			)
			.await
			.expect("the service answers")
	}

	/// Where the service names the events it has stored.
	fn definitions_path() -> String {
		format!(
			"{}/rpc/event_definitions",
			leptoskit_rpc::service_prefix(leptoskit_analytics::SERVICE)
		)
	}

	#[test]
	fn the_month_has_a_funnel_and_comebacks() {
		let batches = month(1_763_000_000_000);
		let names: Vec<&str> = batches
			.iter()
			.flat_map(|batch| batch.entries.iter())
			.filter_map(|entry| {
				match &entry.body {
					Body::TypedEvent { name, .. } | Body::Event { name, .. } => Some(name.as_str()),
					_ => None,
				}
			})
			.collect();
		let count = |name: &str| names.iter().filter(|known| **known == name).count();
		assert_eq!(batches.len(), 30);
		assert!(count("$pageview") > 2_000, "{}", count("$pageview"));
		assert!(count("signed_up") > 200, "{}", count("signed_up"));
		assert!(count("checkout_started") > count("payment_submitted"));
		assert!(count("payment_submitted") > count("checkout_completed"));
		assert!(
			count("checkout_completed") > 5,
			"{}",
			count("checkout_completed")
		);
		assert!(batches.iter().all(|batch| batch.entries.len() < 10_000));
	}

	#[tokio::test]
	async fn seeding_the_service_leaves_a_month_a_token_guards() -> Result<()> {
		const TOKEN: &str = "admin-demo-seed-test-token-not-a-secret-012";
		const MONTH_END: i64 = 1_763_000_000_000;
		let data = scratch("month");
		std::fs::create_dir_all(&data)?;
		let router = start(&data, "http://localhost:4800", TOKEN).await?;

		// The month lands through the service's own routes, and the
		// receipt says so with the service's own numbers: every entry
		// accounted, stored, none refused, none rate-limited, and the
		// events/identity split is not a guess — the service answers for
		// both.
		let receipt = seed_with(router.clone(), TOKEN, MONTH_END).await?;
		assert!(
			receipt.entries > 1_000,
			"{} entries seeded",
			receipt.entries
		);
		assert_eq!(
			receipt.stored + receipt.duplicates,
			receipt.entries,
			"the service stored every entry: {receipt:?}"
		);
		assert_eq!(
			receipt.events + receipt.identities,
			receipt.entries,
			"every entry is an event or an identity: {receipt:?}"
		);
		assert!(
			receipt.events > 1_000 && receipt.identities > 0,
			"a real month has both kinds: {receipt:?}"
		);

		// The summary the admin reads holds the month's events and the
		// schemas the typed events were sent under.
		let answer = ask(&router, &definitions_path(), TOKEN).await;
		let status = answer.status();
		let bytes = axum::body::to_bytes(answer.into_body(), REFUSED_BYTES)
			.await
			.expect("the summary arrives");
		assert_eq!(
			status,
			StatusCode::OK,
			"{}",
			String::from_utf8_lossy(&bytes)
		);
		let summary: Vec<serde_json::Value> = serde_json::from_slice(&bytes)?;
		let named = |name: &str| summary.iter().find(|definition| definition["name"] == name);
		let volume: i64 = summary
			.iter()
			.filter_map(|definition| definition["volume"].as_i64())
			.sum();
		assert!(volume > 1_000, "{summary:?}");
		assert_eq!(
			volume,
			i64::try_from(receipt.entries).expect("a month fits"),
			"the catalogue's volume counts every accepted entry, events and identities alike"
		);
		let signed_up = named("signed_up").expect("signed_up in the summary");
		assert!(
			signed_up["fields"].is_array(),
			"the signed_up schema is registered: {signed_up}"
		);
		assert!(named("$pageview").is_some(), "{summary:?}");

		// A token the service wasn't given reads nothing and seeds nothing.
		let refused = ask(&router, &definitions_path(), "not-the-token").await;
		assert_eq!(refused.status(), StatusCode::FORBIDDEN);
		assert!(
			seed_with(router, "not-the-token", MONTH_END).await.is_err(),
			"the service refuses an unknown token's seed"
		);

		std::fs::remove_dir_all(&data).ok();
		Ok(())
	}

	/// A partially refused seed cannot read as complete: one poisoned
	/// entry in one batch — a uuid that is not a v7, which the service
	/// refuses permanently — makes the whole seed an error, against the
	/// real router and its real typed answer.
	#[tokio::test]
	async fn a_partially_refused_seed_is_an_error_not_a_month() -> Result<()> {
		const TOKEN: &str = "admin-demo-seed-test-token-not-a-secret-012";
		let data = scratch("partial");
		std::fs::create_dir_all(&data)?;
		let router = start(&data, "http://localhost:4800", TOKEN).await?;
		register_with(&router, TOKEN).await?;

		let mut poisoned = month(1_763_000_000_000);
		let mut batch = wire::Batch {
			sent_at: 1_763_000_000_000,
			contexts: Vec::new(),
			entries: Vec::new(),
		};
		// One good entry, one the service must refuse: the answer is a
		// 200 with `refused` non-empty, exactly the partial success that
		// must never publish.
		batch.entries.push(wire::Entry {
			uuid: wire::uuid_v7(1_763_000_000_000, [1_u8; 10]),
			distinct_id: "poisoned-batch".to_owned(),
			session_id: None,
			context: 0,
			body: Body::Event {
				name: "$pageview".to_owned(),
				properties: Vec::new(),
			},
		});
		batch.entries.push(wire::Entry {
			uuid: [0_u8; 16],
			distinct_id: "poisoned-batch".to_owned(),
			session_id: None,
			context: 0,
			body: Body::Event {
				name: "$pageview".to_owned(),
				properties: Vec::new(),
			},
		});
		poisoned.push(batch);

		let outcome = seed_batches(&router, TOKEN, &poisoned).await;
		assert!(outcome.is_err(), "a partially refused seed must fail");
		let error = outcome.expect_err("checked above");
		assert!(
			error.to_string().contains("refused"),
			"the failure names the refusal: {error:#}"
		);

		std::fs::remove_dir_all(&data).ok();
		Ok(())
	}

	/// A re-sent month comes back as duplicates, and the receipt says so
	/// with the service's numbers: `stored` zero, `duplicates` equal to
	/// the entries, and the read-back still counts every row. That is the
	/// truthful shape a fresh generation's exact read-back checks against.
	#[tokio::test]
	async fn a_resent_month_is_reported_as_duplicates_not_new_rows() -> Result<()> {
		const TOKEN: &str = "admin-demo-seed-test-token-not-a-secret-012";
		const MONTH_END: i64 = 1_763_000_000_000;
		let data = scratch("duplicates");
		std::fs::create_dir_all(&data)?;
		let router = start(&data, "http://localhost:4800", TOKEN).await?;

		let first = seed_with(router.clone(), TOKEN, MONTH_END).await?;
		let again = seed_batches(&router, TOKEN, &month(MONTH_END)).await?;

		assert_eq!(again.entries, first.entries, "the same month was re-sent");
		assert_eq!(
			again.stored, 0,
			"nothing was stored a second time: {again:?}"
		);
		assert_eq!(
			again.duplicates, again.entries,
			"every entry answered as a duplicate: {again:?}"
		);

		let answer = ask(&router, &definitions_path(), TOKEN).await;
		let bytes = axum::body::to_bytes(answer.into_body(), REFUSED_BYTES)
			.await
			.expect("the summary arrives");
		let summary: Vec<serde_json::Value> = serde_json::from_slice(&bytes)?;
		let volume: i64 = summary
			.iter()
			.filter_map(|definition| definition["volume"].as_i64())
			.sum();
		assert_eq!(
			volume,
			i64::try_from(first.entries).expect("a month fits"),
			"the re-sent month added no rows"
		);

		std::fs::remove_dir_all(&data).ok();
		Ok(())
	}
}
