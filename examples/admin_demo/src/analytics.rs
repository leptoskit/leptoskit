//! The main site's analytics: the real `leptoskit_analytics` service, its
//! typed events, and a month of traffic to look at.
//!
//! The service runs inside the main site, as `lk dev` would run it beside
//! the app, on its own SQLite database. The admin reads it as the app, with
//! [`TOKEN`]. `--seed` sends 30 days of visits through the same ingestion
//! path a client uses (`POST v1/batch`, signed as the app): sessions from
//! six channels on three kinds of device, sign-ups that identify their
//! anonymous visitor, a checkout funnel that loses people at each step,
//! and people who come back, less each week.

use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use axum::Router;
use leptoskit_analytics::Event;
use leptoskit_analytics::service;
use leptoskit_analytics::service::catalogue::SchemaInput;
use leptoskit_analytics::wire;
use leptoskit_analytics::wire::Body;
use leptoskit_analytics::wire::Value;
use leptoskit_db::Db;
use leptoskit_rpc::client::AppCaller;
use leptoskit_rpc::server::APP_CALLER;
use leptoskit_rpc::server::Anonymous;
use leptoskit_rpc::server::ServiceCallers;

/// The analytics service's token for the app's own callers: the main
/// site's server and the admin. Nothing outside this process accepts it.
pub const TOKEN: &str = "admin-demo-analytics-app-token-not-a-secret-0123";

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

/// The service's routes, its database migrated, its live stream started.
pub async fn start(data: &Path, origin: &str) -> Result<Router> {
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
		ServiceCallers::new(Anonymous).caller(TOKEN, APP_CALLER),
	);
	live.start().await?;
	Ok(router)
}

/// The app's typed events, put in the service's catalogue, as the app's
/// server does when it starts.
pub async fn register(url: &str) -> Result<()> {
	AppCaller::new(leptoskit_analytics::SERVICE, url, TOKEN)
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

/// Sends a month of traffic to the service at `url`, as the app.
pub async fn seed(url: &str) -> Result<usize> {
	register(url).await?;
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
			.bearer_auth(TOKEN)
			.header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
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

#[cfg(test)]
mod tests {
	use super::*;

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
}
