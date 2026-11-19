//! Subscriptions, checkout, the customer portal and the provider's
//! events, written once and run on SQLite (`deployments.rs`) and Postgres
//! (`postgres.rs`). [`Till`] stands in for the payment provider: nothing
//! here reaches Stripe, and every price and ID is made up.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_cloud::billing::SubscriptionStatus;
use leptoskit_cloud::server::Call;
use leptoskit_cloud::server::Checkout;
use leptoskit_cloud::server::ControlConfig;
use leptoskit_cloud::server::NewCustomer;
use leptoskit_cloud::server::PaymentError;
use leptoskit_cloud::server::Payments;
use leptoskit_cloud::server::PaymentsConfig;
use leptoskit_cloud::server::STRIPE_WEBHOOK_PATH;
use leptoskit_cloud::server::Subscription;
use leptoskit_cloud::server::signature_header;
use leptoskit_cloud::tables::AuditEvent;
use leptoskit_cloud::tables::BillingAccount;
use leptoskit_cloud::tables::PaymentEvent;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

use super::App;
use super::As;

/// The secret the tests' provider signs its events with.
pub const WEBHOOK_SECRET: &str = "whsec_made_up_for_the_tests";
/// The provider's price for the tests' `pro` plan.
pub const PRO_PRICE: &str = "price_pro";

#[derive(Debug, Default)]
struct Drawer {
	customers: Vec<(Uuid, String)>,
	subscriptions: HashMap<String, Subscription>,
	charges: HashMap<String, String>,
	checkouts: Vec<(String, String)>,
	portals: Vec<String>,
	stale: Option<Subscription>,
	down: bool,
}

/// A payment provider as a test arranges it.
#[derive(Clone, Debug, Default)]
pub struct Till {
	drawer: Arc<Mutex<Drawer>>,
}

impl Till {
	fn open(&self) -> std::sync::MutexGuard<'_, Drawer> {
		self.drawer.lock().expect("the till")
	}

	/// Whether calls fail from now on.
	pub fn down(&self, down: bool) {
		self.open().down = down;
	}

	/// Every customer made so far.
	pub fn customers(&self) -> Vec<String> {
		self.open()
			.customers
			.iter()
			.map(|(_, id)| id.clone())
			.collect()
	}

	/// Every checkout page handed out: its customer and price.
	pub fn checkouts(&self) -> Vec<(String, String)> {
		self.open().checkouts.clone()
	}

	/// Every customer a portal page was handed out for.
	pub fn portals(&self) -> Vec<String> {
		self.open().portals.clone()
	}

	/// What the provider says the subscription `id` is from now on.
	pub fn subscribe(&self, id: &str, customer: &str, status: SubscriptionStatus, price: &str) {
		let period_end = chrono::DateTime::from_timestamp(1_800_000_000, 0).expect("a time");
		self.subscribe_until(id, customer, status, price, period_end);
	}

	/// [`Self::subscribe`], paid up to `period_end`.
	pub fn subscribe_until(
		&self,
		id: &str,
		customer: &str,
		status: SubscriptionStatus,
		price: &str,
		period_end: leptoskit_db::Timestamp,
	) {
		self.open().subscriptions.insert(
			id.to_owned(),
			Subscription {
				id: id.to_owned(),
				customer: customer.to_owned(),
				status,
				price: Some(price.to_owned()),
				period_end: Some(period_end),
				cancel_at_period_end: false,
			},
		);
	}

	/// Has the next question about a subscription answered with what
	/// `id` is now, however it changes meanwhile: an answer that was
	/// already on its way when the subscription changed.
	pub fn answer_late_once(&self, id: &str) {
		let mut drawer = self.open();
		drawer.stale = drawer.subscriptions.get(id).cloned();
	}

	/// A charge made to `customer`.
	pub fn charge(&self, id: &str, customer: &str) {
		self.open()
			.charges
			.insert(id.to_owned(), customer.to_owned());
	}

	fn up(&self) -> Result<(), PaymentError> {
		if self.open().down {
			Err(PaymentError::Unavailable)
		} else {
			Ok(())
		}
	}
}

impl Payments for Till {
	fn customer(&self, new: NewCustomer) -> Call<'_, String> {
		Box::pin(async move {
			self.up()?;
			let mut drawer = self.open();

			if let Some((_, id)) = drawer
				.customers
				.iter()
				.find(|(organization, _)| *organization == new.organization)
			{
				return Ok(id.clone());
			}

			let id = format!("cus_{}", drawer.customers.len() + 1);
			drawer.customers.push((new.organization, id.clone()));
			Ok(id)
		})
	}

	fn checkout<'a>(&'a self, checkout: Checkout<'a>) -> Call<'a, String> {
		Box::pin(async move {
			self.up()?;
			let mut drawer = self.open();
			drawer
				.checkouts
				.push((checkout.customer.to_owned(), checkout.price.to_owned()));
			assert!(checkout.success_url.contains("checkout=done"));
			Ok(format!(
				"https://pay.example.test/checkout/{}",
				drawer.checkouts.len()
			))
		})
	}

	fn portal<'a>(&'a self, customer: &'a str, _return_url: &'a str) -> Call<'a, String> {
		Box::pin(async move {
			self.up()?;
			self.open().portals.push(customer.to_owned());
			Ok(format!("https://pay.example.test/portal/{customer}"))
		})
	}

	fn subscription<'a>(&'a self, id: &'a str) -> Call<'a, Subscription> {
		Box::pin(async move {
			self.up()?;
			let mut drawer = self.open();

			if let Some(stale) = drawer.stale.take_if(|stale| stale.id == id) {
				return Ok(stale);
			}

			drawer
				.subscriptions
				.get(id)
				.cloned()
				.ok_or(PaymentError::NotFound)
		})
	}

	fn charge_customer<'a>(&'a self, id: &'a str) -> Call<'a, Option<String>> {
		Box::pin(async move {
			self.up()?;
			Ok(self.open().charges.get(id).cloned())
		})
	}
}

/// A control plane's configuration that takes payment through `till`, with
/// the tests' catalogue: `hobby` is free and `pro` is sold.
pub fn taking_payment(till: &Till) -> impl FnOnce(&mut ControlConfig) + use<> {
	let till = till.clone();
	move |config| {
		config.catalogue = Some(super::billing::catalogue());
		config.payments = Some(PaymentsConfig {
			provider: Arc::new(till),
			webhook_secret: WEBHOOK_SECRET.to_owned(),
			livemode: false,
			prices: BTreeMap::from([("pro".to_owned(), PRO_PRICE.to_owned())]),
			automatic_tax: false,
		});
	}
}

/// Posts `body` to the provider's route, signed with `secret` now.
async fn post(app: &App, secret: &str, body: &Value) -> u16 {
	let body = body.to_string().into_bytes();
	let header = signature_header(secret, leptoskit_db::now().timestamp(), &body);
	app.client
		.post(format!("{}{STRIPE_WEBHOOK_PATH}", app.origin))
		.header("stripe-signature", header)
		.body(body)
		.send()
		.await
		.expect("the server answers")
		.status()
		.as_u16()
}

/// Sends the event `id` of `kind` about `object`, signed as the provider
/// signs.
async fn send(app: &App, id: &str, kind: &str, object: Value) -> u16 {
	post(
		app,
		WEBHOOK_SECRET,
		&json!({ "id": id, "type": kind, "data": { "object": object } }),
	)
	.await
}

async fn audited(app: &App, action: &str) -> usize {
	AuditEvent::query()
		.filter(|event| event.action.eq(action))
		.count(&app.system())
		.await
		.expect("audit")
		.try_into()
		.expect("a count")
}

async fn events(app: &App) -> usize {
	PaymentEvent::query()
		.count(&app.system())
		.await
		.expect("events")
		.try_into()
		.expect("a count")
}

/// An organisation's plan follows what the provider says its subscription
/// is, and nothing else: not a checkout that was only started, not what an
/// event's body claims, and not an event nobody signed.
#[expect(
	clippy::cognitive_complexity,
	clippy::too_many_lines,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_plan_follows_the_providers_subscription(app: &App, till: &Till) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let subscription = || {
		let caller = As::Session(&ana);
		let org = org.clone();
		async move {
			app.call("subscription", json!({ "organization": org }), caller)
				.await
				.expect(200)
				.json()
		}
	};
	let checkout = |plan: &str, caller| {
		app.call(
			"checkout",
			json!({ "organization": org, "plan": plan }),
			caller,
		)
	};
	let portal = |caller| app.call("portal", json!({ "organization": org }), caller);

	let fresh = subscription().await;
	assert_eq!(
		fresh,
		json!({
			"plan": "hobby",
			"payments": true,
			"sold": ["pro"],
			"status": null,
			"period_end": null,
			"cancel_at_period_end": false,
		})
	);
	// Nothing to manage yet, and only what is sold can be bought.
	portal(As::Session(&ana)).await.refused(409, "no_customer");
	checkout("hobby", As::Session(&ana))
		.await
		.refused(409, "plan_not_sold");
	checkout("enterprise", As::Session(&ana))
		.await
		.refused(404, "plan_not_found");
	// Only a person who may manage the bill starts one.
	let dev = app
		.join(As::Session(&ana), &org, "dev@example.com", "developer")
		.await;
	checkout("pro", As::Session(&dev))
		.await
		.refused(403, "insufficient_role");
	portal(As::Session(&dev))
		.await
		.refused(403, "insufficient_role");
	let (token, _) = app.token(As::Session(&ana), &org, &["billing"], None).await;
	checkout("pro", As::Bearer(&token))
		.await
		.refused(403, "person_required");
	portal(As::Bearer(&token))
		.await
		.refused(403, "person_required");
	assert_eq!(till.customers(), Vec::<String>::new());

	// The provider being away is said, and nothing is kept.
	till.down(true);
	checkout("pro", As::Session(&ana))
		.await
		.refused(409, "payments_unavailable");
	till.down(false);

	// A checkout is a page at the provider. Starting one changes no plan.
	let started = checkout("pro", As::Session(&ana)).await;
	let url = started.expect(200).json()["url"]
		.as_str()
		.expect("a url")
		.to_owned();
	assert!(
		url.starts_with("https://pay.example.test/checkout/"),
		"{url}"
	);
	assert_eq!(till.customers(), ["cus_1"]);
	assert_eq!(
		till.checkouts(),
		[("cus_1".to_owned(), PRO_PRICE.to_owned())]
	);
	assert_eq!(subscription().await["plan"], "hobby");
	// Another checkout is the same customer's.
	checkout("pro", As::Session(&ana)).await.expect(200);
	assert_eq!(till.customers(), ["cus_1"]);
	assert_eq!(audited(app, "billing.checkout").await, 2);

	// An event nobody signed, or signed with another secret, is refused
	// before anything is asked or written.
	till.subscribe("sub_1", "cus_1", SubscriptionStatus::Active, PRO_PRICE);
	let created = json!({
		"id": "evt_1",
		"type": "customer.subscription.created",
		"data": { "object": { "id": "sub_1" } },
	});
	assert_eq!(post(app, "whsec_someone_elses", &created).await, 400);
	let unsigned = app
		.client
		.post(format!("{}{STRIPE_WEBHOOK_PATH}", app.origin))
		.body(created.to_string())
		.send()
		.await
		.expect("the server answers");
	assert_eq!(unsigned.status().as_u16(), 400);
	// So is one about live money, here where the key is a test one.
	let mut real_money = created.clone();
	real_money["livemode"] = json!(true);
	assert_eq!(post(app, WEBHOOK_SECRET, &real_money).await, 400);
	assert_eq!(subscription().await["plan"], "hobby");
	assert_eq!(events(app).await, 0);

	// Signed, it makes the organisation follow the subscription.
	assert_eq!(post(app, WEBHOOK_SECRET, &created).await, 200);
	let active = subscription().await;
	assert_eq!(active["plan"], "pro");
	assert_eq!(active["status"], "active");
	assert_eq!(active["period_end"], "2027-01-15T08:00:00Z");
	assert_eq!(audited(app, "billing.plan_changed").await, 1);
	// Sent again, it is dealt with once.
	assert_eq!(post(app, WEBHOOK_SECRET, &created).await, 200);
	assert_eq!(events(app).await, 1);
	assert_eq!(audited(app, "billing.plan_changed").await, 1);

	// While it runs there is nothing to check out; the portal changes it.
	checkout("pro", As::Session(&ana))
		.await
		.refused(409, "already_subscribed");
	let page = portal(As::Session(&ana)).await;
	assert_eq!(
		page.expect(200).json()["url"],
		"https://pay.example.test/portal/cus_1"
	);
	assert_eq!(till.portals(), ["cus_1"]);

	// What an event's body claims decides nothing: the provider is asked.
	let lie = json!({ "id": "sub_1", "status": "canceled", "customer": "cus_1" });
	assert_eq!(
		send(app, "evt_2", "customer.subscription.deleted", lie).await,
		200
	);
	assert_eq!(subscription().await["plan"], "pro");

	// A payment that failed and is being tried again keeps the plan.
	till.subscribe("sub_1", "cus_1", SubscriptionStatus::PastDue, PRO_PRICE);
	let invoice = json!({
		"id": "in_1",
		"parent": { "subscription_details": { "subscription": "sub_1" } },
	});
	assert_eq!(
		send(app, "evt_3", "invoice.payment_failed", invoice).await,
		200
	);
	let late = subscription().await;
	assert_eq!(
		(&late["plan"], &late["status"]),
		(&json!("pro"), &json!("past_due"))
	);

	// An older subscription of the same customer ending says nothing
	// about the one the organisation follows.
	till.subscribe("sub_0", "cus_1", SubscriptionStatus::Canceled, PRO_PRICE);
	assert_eq!(
		send(
			app,
			"evt_4",
			"customer.subscription.deleted",
			json!({ "id": "sub_0" })
		)
		.await,
		200
	);
	assert_eq!(subscription().await["plan"], "pro");

	// With the provider away the event is asked for again, and isn't
	// counted as dealt with.
	till.subscribe("sub_1", "cus_1", SubscriptionStatus::Canceled, PRO_PRICE);
	till.down(true);
	let ended = json!({ "id": "sub_1" });
	assert_eq!(
		send(app, "evt_5", "customer.subscription.deleted", ended.clone()).await,
		503
	);
	assert_eq!(subscription().await["plan"], "pro");
	assert_eq!(events(app).await, 4);
	till.down(false);
	// The subscription over, the organisation is back where everyone starts.
	assert_eq!(
		send(app, "evt_5", "customer.subscription.deleted", ended).await,
		200
	);
	let over = subscription().await;
	assert_eq!(
		(&over["plan"], &over["status"]),
		(&json!("hobby"), &json!("canceled"))
	);
	assert_eq!(audited(app, "billing.plan_changed").await, 2);

	// A subscription to a price no plan here has changes no plan, and
	// says so in the audit log.
	till.subscribe(
		"sub_2",
		"cus_1",
		SubscriptionStatus::Active,
		"price_unheard_of",
	);
	assert_eq!(
		send(
			app,
			"evt_6",
			"customer.subscription.created",
			json!({ "id": "sub_2" })
		)
		.await,
		200
	);
	let unsold = subscription().await;
	assert_eq!(
		(&unsold["plan"], &unsold["status"]),
		(&json!("hobby"), &json!("canceled"))
	);
	assert_eq!(audited(app, "billing.unknown_price").await, 1);

	// Money coming back out is noted against the organisation whose
	// charge the provider says it was, whatever the event says.
	till.charge("ch_1", "cus_1");
	assert_eq!(
		send(
			app,
			"evt_7",
			"charge.dispute.created",
			json!({ "id": "dp_1", "charge": "ch_1" })
		)
		.await,
		200
	);
	assert_eq!(audited(app, "billing.dispute").await, 1);
	assert_eq!(
		send(
			app,
			"evt_7b",
			"charge.refunded",
			json!({ "id": "ch_1", "customer": "cus_2" })
		)
		.await,
		200
	);
	assert_eq!(audited(app, "billing.refund").await, 1);
	// A note that can't be placed while the provider is away is let go:
	// it would change nothing.
	till.down(true);
	assert_eq!(
		send(app, "evt_7c", "charge.refunded", json!({ "id": "ch_1" })).await,
		200
	);
	till.down(false);
	assert_eq!(audited(app, "billing.refund").await, 1);

	// Somebody else's customer, a subscription the provider doesn't have,
	// and a kind of event nothing here follows: all dealt with, by doing
	// nothing.
	till.subscribe("sub_9", "cus_999", SubscriptionStatus::Active, PRO_PRICE);
	for (id, kind, object) in [
		(
			"evt_8",
			"customer.subscription.created",
			json!({ "id": "sub_9" }),
		),
		(
			"evt_9",
			"customer.subscription.updated",
			json!({ "id": "sub_gone" }),
		),
		("evt_10", "customer.created", json!({ "id": "cus_1" })),
	] {
		assert_eq!(send(app, id, kind, object).await, 200, "{id}");
	}
	assert_eq!(subscription().await["plan"], "hobby");

	// Nobody outside sees any of it, and another organisation's checkout
	// is another customer.
	let mallory = app.sign_in("mallory@example.com").await;
	let answer = app
		.call(
			"subscription",
			json!({ "organization": org }),
			As::Session(&mallory),
		)
		.await;
	assert_eq!(answer.status, 404, "{}", answer.text);
	checkout("pro", As::Session(&mallory))
		.await
		.refused(404, "organization_not_found");
	let theirs = app.personal(As::Session(&mallory)).await;
	app.call(
		"checkout",
		json!({ "organization": theirs, "plan": "pro" }),
		As::Session(&mallory),
	)
	.await
	.expect(200);
	assert_eq!(till.customers(), ["cus_1", "cus_2"]);
	let accounts = BillingAccount::query()
		.all(&app.system())
		.await
		.expect("accounts");
	assert_eq!(accounts.len(), 2);
	// The signing secret is the configuration's alone.
	assert_eq!(
		app.find_in_every_table(WEBHOOK_SECRET).await,
		Vec::<String>::new()
	);
}

/// A control plane that takes no payment sells nothing, and has no route
/// for a provider's events.
pub async fn without_payment_nothing_is_sold(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let summary = app
		.call(
			"subscription",
			json!({ "organization": org }),
			As::Session(&ana),
		)
		.await;
	let summary = summary.expect(200).json();
	assert_eq!(
		(&summary["payments"], &summary["sold"]),
		(&json!(false), &json!([]))
	);
	app.call(
		"checkout",
		json!({ "organization": org, "plan": "pro" }),
		As::Session(&ana),
	)
	.await
	.refused(409, "payments_unavailable");
	app.call("portal", json!({ "organization": org }), As::Session(&ana))
		.await
		.refused(409, "payments_unavailable");
	let event = json!({ "id": "evt_1", "type": "x", "data": { "object": {} } });
	assert_eq!(post(app, WEBHOOK_SECRET, &event).await, 404);
}

/// What the provider says is caught up with when its events arrive out of
/// order or not at all: an answer that was already old when it was written
/// is replaced, the sweeper asks about a subscription whose paid time ran
/// out in silence, and a second subscription beside a running one changes
/// nothing.
#[expect(
	clippy::cognitive_complexity,
	clippy::too_many_lines,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_missed_event_is_caught_up_with(app: &App, till: &Till) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let subscription = || {
		let caller = As::Session(&ana);
		let org = org.clone();
		async move {
			app.call("subscription", json!({ "organization": org }), caller)
				.await
				.expect(200)
				.json()
		}
	};
	app.call(
		"checkout",
		json!({ "organization": org, "plan": "pro" }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	// Paid up to two hours ago: the provider renews it, and here that
	// event never arrives.
	let lapsed = leptoskit_db::now() - chrono::TimeDelta::hours(2);
	till.subscribe_until(
		"sub_1",
		"cus_1",
		SubscriptionStatus::Active,
		PRO_PRICE,
		lapsed,
	);
	let about = |id: &str| json!({ "id": id });
	assert_eq!(
		send(
			app,
			"evt_1",
			"customer.subscription.created",
			about("sub_1")
		)
		.await,
		200
	);
	assert_eq!(subscription().await["plan"], "pro");

	// A second subscription beside the running one changes nothing, and
	// is noted.
	till.subscribe("sub_2", "cus_1", SubscriptionStatus::Active, PRO_PRICE);
	assert_eq!(
		send(
			app,
			"evt_2",
			"customer.subscription.created",
			about("sub_2")
		)
		.await,
		200
	);
	assert_eq!(audited(app, "billing.second_subscription").await, 1);
	let account = BillingAccount::query()
		.one(&app.system())
		.await
		.expect("an account");
	assert_eq!(account.subscription.as_deref(), Some("sub_1"));

	// The renewal nobody told us about: the sweeper asks.
	till.subscribe("sub_1", "cus_1", SubscriptionStatus::Active, PRO_PRICE);
	let swept = app.control.sweep().await.expect("the sweep runs");
	assert_eq!(swept.subscriptions, 1);
	let renewed = subscription().await;
	assert_eq!(
		(&renewed["plan"], &renewed["period_end"]),
		(&json!("pro"), &json!("2027-01-15T08:00:00Z"))
	);
	// Paid up, there is nothing to ask.
	let swept = app.control.sweep().await.expect("the sweep runs");
	assert_eq!(swept.subscriptions, 0);

	// An answer already on its way when the subscription ended is written,
	// and then put right: the provider is asked once more.
	till.answer_late_once("sub_1");
	till.subscribe("sub_1", "cus_1", SubscriptionStatus::Canceled, PRO_PRICE);
	till.subscribe("sub_2", "cus_1", SubscriptionStatus::Canceled, PRO_PRICE);
	assert_eq!(
		send(
			app,
			"evt_3",
			"customer.subscription.deleted",
			about("sub_1")
		)
		.await,
		200
	);
	let over = subscription().await;
	assert_eq!(
		(&over["plan"], &over["status"]),
		(&json!("hobby"), &json!("canceled"))
	);

	// A cancellation nobody told us about, on a subscription whose paid
	// time is over: the sweeper finds that too.
	till.subscribe_until(
		"sub_3",
		"cus_1",
		SubscriptionStatus::Active,
		PRO_PRICE,
		lapsed,
	);
	assert_eq!(
		send(
			app,
			"evt_4",
			"customer.subscription.created",
			about("sub_3")
		)
		.await,
		200
	);
	assert_eq!(subscription().await["plan"], "pro");
	till.subscribe_until(
		"sub_3",
		"cus_1",
		SubscriptionStatus::Canceled,
		PRO_PRICE,
		lapsed,
	);
	// The provider away, the plan stays until it can be asked.
	till.down(true);
	app.control.sweep().await.expect("the sweep runs");
	assert_eq!(subscription().await["plan"], "pro");
	till.down(false);
	let swept = app.control.sweep().await.expect("the sweep runs");
	assert_eq!(swept.subscriptions, 1);
	assert_eq!(subscription().await["plan"], "hobby");

	// A plan an operator put the organisation on by agreement isn't the
	// provider's to take away.
	till.subscribe("sub_4", "cus_1", SubscriptionStatus::Active, PRO_PRICE);
	assert_eq!(
		send(
			app,
			"evt_5",
			"customer.subscription.created",
			about("sub_4")
		)
		.await,
		200
	);
	let slug = org.clone();
	leptoskit_cloud::tables::Organization::update()
		.filter(|organization| organization.slug.eq(slug.as_str()))
		.set(|organization| organization.plan, "agreed".to_owned())
		.run(&app.system())
		.await
		.expect("change the plan");
	till.subscribe("sub_4", "cus_1", SubscriptionStatus::Canceled, PRO_PRICE);
	assert_eq!(
		send(
			app,
			"evt_6",
			"customer.subscription.deleted",
			about("sub_4")
		)
		.await,
		200
	);
	let agreed = subscription().await;
	assert_eq!(
		(&agreed["plan"], &agreed["status"]),
		(&json!("agreed"), &json!("canceled"))
	);
}
