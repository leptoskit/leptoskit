//! Taking payment: the few questions the control plane asks of a payment
//! provider, and the events the provider sends back.
//!
//! The control plane sells one thing through the provider: a plan, as a
//! subscription. It asks for a customer, a hosted checkout page, a hosted
//! page to manage the subscription, and what a subscription is now. It
//! never sees a card. [`Stripe`] asks Stripe's API; tests put their own
//! [`Payments`] in its place.
//!
//! What the provider says happened arrives as signed events
//! ([`verify_event`]). An event is only ever a reason to ask the provider
//! what is true now: nothing in an event's body decides a plan.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use hmac::Mac as _;
use leptoskit_db::Timestamp;
use serde_json::Value;
use subtle::ConstantTimeEq as _;
use uuid::Uuid;

use crate::billing::SubscriptionStatus;

/// The version of Stripe's API every request names.
pub const STRIPE_API_VERSION: &str = "2026-10-28.endive";

/// The label checkout sessions carry in Stripe's dashboard.
const INTEGRATION_IDENTIFIER: &str = "leptoskit_cloud_plan_qhzrtwmk";

/// How long one call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// The most of an answer that is read.
const MAX_ANSWER_LEN: usize = 1024 * 1024;

/// The most of an event that is read.
pub(crate) const MAX_EVENT_LEN: usize = 1024 * 1024;

/// How old an event's signature may be, in seconds.
pub(crate) const SIGNATURE_TOLERANCE: i64 = 300;

/// The longest ID of the provider's that is kept.
pub(crate) const MAX_ID_LEN: usize = 255;

type HmacSha256 = hmac::Hmac<sha2::Sha256>;

/// Why a call to the provider gave no answer. Never the reason in detail:
/// it could carry what the provider said about a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PaymentError {
	/// The provider couldn't be reached, or didn't answer as expected.
	#[error("the payment provider couldn't be reached")]
	Unavailable,
	/// The provider has no such thing.
	#[error("the payment provider has no such record")]
	NotFound,
}

/// What a call answers with.
pub type Call<'a, T> = Pin<Box<dyn Future<Output = Result<T, PaymentError>> + Send + 'a>>;

/// An organisation, as the provider's customer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewCustomer {
	/// The organisation's ID, and all the provider is told about it: the
	/// same customer is answered for it however many times it is asked for,
	/// whatever the organisation is called by then.
	pub organization: Uuid,
}

/// A checkout page for one plan's subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkout<'a> {
	pub customer: &'a str,
	/// The provider's price for the plan.
	pub price: &'a str,
	pub organization: Uuid,
	pub success_url: &'a str,
	pub cancel_url: &'a str,
	/// Whether the provider works out tax.
	pub automatic_tax: bool,
}

/// A subscription, as the provider has it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
	pub id: String,
	pub customer: String,
	pub status: SubscriptionStatus,
	/// The price of its first item.
	pub price: Option<String>,
	/// When what is paid for ends.
	pub period_end: Option<Timestamp>,
	/// Whether it ends then instead of renewing.
	pub cancel_at_period_end: bool,
}

/// Something that takes payment.
pub trait Payments: Send + Sync + std::fmt::Debug {
	/// The customer for an organisation, made if there is none: its ID.
	fn customer(&self, new: NewCustomer) -> Call<'_, String>;
	/// A hosted page that starts a subscription: its address.
	fn checkout<'a>(&'a self, checkout: Checkout<'a>) -> Call<'a, String>;
	/// A hosted page where `customer` manages their subscription, payment
	/// method and invoices: its address.
	fn portal<'a>(&'a self, customer: &'a str, return_url: &'a str) -> Call<'a, String>;
	/// The subscription `id`, as it is now.
	fn subscription<'a>(&'a self, id: &'a str) -> Call<'a, Subscription>;
	/// The customer the charge `id` was made to, when it has one.
	fn charge_customer<'a>(&'a self, id: &'a str) -> Call<'a, Option<String>>;
	/// What it was configured with that must never reach a log: its key.
	fn secrets(&self) -> Vec<String> {
		Vec::new()
	}
}

/// A [`Payments`] the control plane shares.
pub type SharedPayments = Arc<dyn Payments>;

/// How the control plane takes payment.
#[derive(Clone)]
pub struct PaymentsConfig {
	/// What is asked.
	pub provider: SharedPayments,
	/// The secret the provider signs its events with.
	pub webhook_secret: String,
	/// Whether the provider's key is a live one. An event from the other
	/// mode is refused: a test event never changes what a live organisation
	/// is on.
	pub livemode: bool,
	/// The provider's price for each plan that is sold, by the plan's slug.
	/// A plan without one can't be subscribed to here.
	pub prices: BTreeMap<String, String>,
	/// Whether checkout asks the provider to work out tax. It collects none
	/// until the provider's account is registered where tax is owed.
	pub automatic_tax: bool,
}

impl std::fmt::Debug for PaymentsConfig {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("PaymentsConfig")
			.field("provider", &self.provider)
			.field("livemode", &self.livemode)
			.field("prices", &self.prices)
			.field("automatic_tax", &self.automatic_tax)
			.finish_non_exhaustive()
	}
}

impl PaymentsConfig {
	/// The plan `price` is the provider's price for.
	pub(crate) fn plan_of(&self, price: &str) -> Option<&str> {
		self.prices
			.iter()
			.find(|(_, candidate)| candidate.as_str() == price)
			.map(|(plan, _)| plan.as_str())
	}
}

/// Whether `id` is one of the provider's IDs: short, and of letters,
/// digits and underscores, so it is safe in a path and a row.
pub(crate) fn is_id(id: &str) -> bool {
	!id.is_empty()
		&& id.len() <= MAX_ID_LEN
		&& id
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Sanitizes an error token (`type`, `code`, `request-id`) for operator logs:
/// at most 64 ASCII alphanumeric, `_`, or `-` characters.
fn sanitize_token(raw: &str) -> String {
	let cleaned: String = raw
		.chars()
		.filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
		.take(64)
		.collect();
	if cleaned.is_empty() {
		"unknown".to_owned()
	} else {
		cleaned
	}
}

/// Stripe, over its HTTP API.
#[derive(Clone)]
pub struct Stripe {
	key: String,
	base: reqwest::Url,
	client: reqwest::Client,
}

impl std::fmt::Debug for Stripe {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Stripe")
			.field("base", &self.base.as_str())
			.finish_non_exhaustive()
	}
}

impl Stripe {
	/// Stripe, with `key`: a restricted key that may write customers,
	/// checkout sessions and portal sessions, and read subscriptions and
	/// charges.
	///
	/// # Errors
	///
	/// When `key` isn't a secret or restricted key.
	pub fn new(key: &str) -> Result<Self, String> {
		Self::at("https://api.stripe.com", key)
	}

	/// Whether `key` is a live key, as Stripe names them (`rk_live_…`).
	#[must_use]
	pub fn is_live_key(key: &str) -> bool {
		key.split('_').nth(1) == Some("live")
	}

	/// Stripe's API at another address, for a local stand-in.
	///
	/// # Errors
	///
	/// When `base` isn't an `https` address, or an `http` one on this
	/// machine; when it carries a user, a password or a query; or when
	/// `key` isn't a secret or restricted key.
	pub fn at(base: &str, key: &str) -> Result<Self, String> {
		let base =
			reqwest::Url::parse(base).map_err(|_| "Stripe's address isn't a URL".to_owned())?;
		let local = matches!(
			base.host(),
			Some(
				url::Host::Domain("localhost")
					| url::Host::Ipv4(std::net::Ipv4Addr::LOCALHOST)
					| url::Host::Ipv6(std::net::Ipv6Addr::LOCALHOST)
			)
		);

		// The key goes to this address with every call.
		if !(base.scheme() == "https" || base.scheme() == "http" && local) {
			return Err("Stripe's address must be https, or http on this machine".to_owned());
		}

		if base.query().is_some() || !base.username().is_empty() || base.password().is_some() {
			return Err("Stripe's address can't carry a user, a password or a query".to_owned());
		}

		if !(key.starts_with("rk_") || key.starts_with("sk_"))
			|| key.len() < 10
			|| !key.bytes().all(|byte| byte.is_ascii_graphic())
		{
			return Err(
				"the Stripe key must be a restricted key (`rk_…`) or a secret key (`sk_…`)"
					.to_owned(),
			);
		}

		let client = reqwest::Client::builder()
			.timeout(CALL_TIMEOUT)
			.redirect(reqwest::redirect::Policy::none())
			.build()
			.map_err(|_| "can't make an HTTP client".to_owned())?;
		Ok(Self {
			key: key.to_owned(),
			base,
			client,
		})
	}

	fn address(&self, path: &str) -> Result<reqwest::Url, PaymentError> {
		self.base.join(path).map_err(|_| PaymentError::Unavailable)
	}

	async fn send(&self, request: reqwest::RequestBuilder) -> Result<Value, PaymentError> {
		let mut response = request
			.bearer_auth(&self.key)
			.header("stripe-version", STRIPE_API_VERSION)
			.send()
			.await
			.map_err(|_| PaymentError::Unavailable)?;

		if response.status() == reqwest::StatusCode::NOT_FOUND {
			return Err(PaymentError::NotFound);
		}

		let status = response.status();

		if !status.is_success() {
			let request_id = response
				.headers()
				.get("request-id")
				.and_then(|value| value.to_str().ok())
				.map_or_else(|| "none".to_owned(), sanitize_token);
			let mut err_body = Vec::new();
			const MAX_ERR_BODY: usize = 16 * 1024;

			while let Some(chunk) = response.chunk().await.ok().flatten() {
				let take = chunk.len().min(MAX_ERR_BODY.saturating_sub(err_body.len()));
				err_body.extend_from_slice(&chunk[..take]);
				if err_body.len() >= MAX_ERR_BODY {
					break;
				}
			}

			let parsed = serde_json::from_slice::<Value>(&err_body).ok();
			let kind = parsed
				.as_ref()
				.and_then(|v| v.pointer("/error/type").and_then(Value::as_str))
				.map_or_else(|| "unknown".to_owned(), sanitize_token);
			let code = parsed
				.as_ref()
				.and_then(|v| v.pointer("/error/code").and_then(Value::as_str))
				.map_or_else(|| "none".to_owned(), sanitize_token);
			eprintln!(
				"leptoskit-cloud: stripe answered {} ({kind}/{code}, request-id={request_id})",
				status.as_u16()
			);
			return Err(PaymentError::Unavailable);
		}

		let mut body = Vec::new();

		while let Some(chunk) = response
			.chunk()
			.await
			.map_err(|_| PaymentError::Unavailable)?
		{
			if body.len() + chunk.len() > MAX_ANSWER_LEN {
				return Err(PaymentError::Unavailable);
			}

			body.extend_from_slice(&chunk);
		}

		serde_json::from_slice(&body).map_err(|_| PaymentError::Unavailable)
	}

	async fn post(
		&self,
		path: &str,
		form: &[(&str, &str)],
		idempotency_key: Option<&str>,
	) -> Result<Value, PaymentError> {
		let body = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs(form)
			.finish();
		let mut request = self
			.client
			.post(self.address(path)?)
			.header("content-type", "application/x-www-form-urlencoded")
			.body(body);

		if let Some(key) = idempotency_key {
			request = request.header("idempotency-key", key);
		}

		self.send(request).await
	}

	async fn get(&self, path: &str) -> Result<Value, PaymentError> {
		self.send(self.client.get(self.address(path)?)).await
	}

	async fn make_customer(&self, new: NewCustomer) -> Result<String, PaymentError> {
		let organization = new.organization.to_string();
		// The key names the organisation and the form says nothing else,
		// so the same key always carries the same form.
		let answer = self
			.post(
				"/v1/customers",
				&customer_form(&organization),
				Some(&format!("leptoskit-cloud-customer-{organization}")),
			)
			.await?;
		id_in(&answer, "id")
	}

	async fn make_checkout(&self, checkout: Checkout<'_>) -> Result<String, PaymentError> {
		let organization = checkout.organization.to_string();
		let answer = self
			.post(
				"/v1/checkout/sessions",
				&checkout_form(&checkout, &organization),
				None,
			)
			.await?;
		url_in(&answer)
	}

	async fn make_portal(&self, customer: &str, return_url: &str) -> Result<String, PaymentError> {
		let answer = self
			.post(
				"/v1/billing_portal/sessions",
				&[("customer", customer), ("return_url", return_url)],
				None,
			)
			.await?;
		url_in(&answer)
	}

	async fn read_subscription(&self, id: &str) -> Result<Subscription, PaymentError> {
		if !is_id(id) {
			return Err(PaymentError::NotFound);
		}

		let answer = self.get(&format!("/v1/subscriptions/{id}")).await?;
		parse_subscription(&answer).ok_or(PaymentError::Unavailable)
	}

	async fn read_charge_customer(&self, id: &str) -> Result<Option<String>, PaymentError> {
		if !is_id(id) {
			return Err(PaymentError::NotFound);
		}

		let answer = self.get(&format!("/v1/charges/{id}")).await?;
		Ok(reference(&answer["customer"]))
	}
}

impl Payments for Stripe {
	fn customer(&self, new: NewCustomer) -> Call<'_, String> {
		Box::pin(self.make_customer(new))
	}

	fn checkout<'a>(&'a self, checkout: Checkout<'a>) -> Call<'a, String> {
		Box::pin(self.make_checkout(checkout))
	}

	fn portal<'a>(&'a self, customer: &'a str, return_url: &'a str) -> Call<'a, String> {
		Box::pin(self.make_portal(customer, return_url))
	}

	fn subscription<'a>(&'a self, id: &'a str) -> Call<'a, Subscription> {
		Box::pin(self.read_subscription(id))
	}

	fn charge_customer<'a>(&'a self, id: &'a str) -> Call<'a, Option<String>> {
		Box::pin(self.read_charge_customer(id))
	}

	fn secrets(&self) -> Vec<String> {
		vec![self.key.clone()]
	}
}

fn customer_form(organization: &str) -> [(&str, &str); 1] {
	[("metadata[organization]", organization)]
}

/// A subscription checkout: one plan, once. No payment method types are
/// named, so the provider's own settings choose them.
fn checkout_form<'a>(checkout: &Checkout<'a>, organization: &'a str) -> Vec<(&'a str, &'a str)> {
	let mut form = vec![
		("mode", "subscription"),
		("customer", checkout.customer),
		("client_reference_id", organization),
		("line_items[0][price]", checkout.price),
		("line_items[0][quantity]", "1"),
		("success_url", checkout.success_url),
		("cancel_url", checkout.cancel_url),
		("integration_identifier", INTEGRATION_IDENTIFIER),
	];

	if checkout.automatic_tax {
		form.push(("automatic_tax[enabled]", "true"));
		// Tax needs an address, and the checkout page is where one is given.
		form.push(("customer_update[address]", "auto"));
	}

	form
}

fn id_in(answer: &Value, field: &str) -> Result<String, PaymentError> {
	answer[field]
		.as_str()
		.filter(|id| is_id(id))
		.map(str::to_owned)
		.ok_or(PaymentError::Unavailable)
}

fn url_in(answer: &Value) -> Result<String, PaymentError> {
	answer["url"]
		.as_str()
		.filter(|url| url.starts_with("https://") && url.len() <= 2048)
		.map(str::to_owned)
		.ok_or(PaymentError::Unavailable)
}

/// An ID where the provider sends either the ID or the whole object.
pub(crate) fn reference(value: &Value) -> Option<String> {
	value
		.as_str()
		.or_else(|| value["id"].as_str())
		.filter(|id| is_id(id))
		.map(str::to_owned)
}

fn seconds(value: &Value) -> Option<Timestamp> {
	chrono::DateTime::from_timestamp(value.as_i64()?, 0)
}

/// A subscription from the provider's JSON. The period is the item's on
/// current versions of the API, and the subscription's on older ones.
pub(crate) fn parse_subscription(answer: &Value) -> Option<Subscription> {
	let item = &answer["items"]["data"][0];
	Some(Subscription {
		id: reference(&answer["id"])?,
		customer: reference(&answer["customer"])?,
		status: SubscriptionStatus::parse(answer["status"].as_str()?),
		price: reference(&item["price"]),
		period_end: seconds(&item["current_period_end"])
			.or_else(|| seconds(&answer["current_period_end"])),
		cancel_at_period_end: answer["cancel_at_period_end"].as_bool().unwrap_or(false),
	})
}

/// An event the provider sent, once its signature is checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Event {
	pub(crate) id: String,
	pub(crate) kind: String,
	/// Whether it is about live money. An event that doesn't say is taken
	/// as a test one.
	pub(crate) livemode: bool,
	/// The object the event is about.
	pub(crate) object: Value,
}

/// Why an event wasn't taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventError {
	/// Its signature is missing, wrong, or too old.
	Signature,
	/// It is signed, and isn't an event.
	Malformed,
}

fn sign(secret: &str, timestamp: &str, body: &[u8]) -> String {
	let mut mac = <HmacSha256 as hmac::Mac>::new_from_slice(secret.as_bytes())
		.unwrap_or_else(|_| unreachable!("HMAC takes a key of any length"));
	mac.update(timestamp.as_bytes());
	mac.update(b".");
	mac.update(body);
	hex::encode(mac.finalize().into_bytes())
}

/// The `Stripe-Signature` header for `body` signed at `timestamp`, as the
/// provider writes it. For stand-ins and tests.
#[must_use]
pub fn signature_header(secret: &str, timestamp: i64, body: &[u8]) -> String {
	let timestamp = timestamp.to_string();
	format!("t={timestamp},v1={}", sign(secret, &timestamp, body))
}

/// The event in `body`, when `header` is a signature of it by `secret`
/// made within [`SIGNATURE_TOLERANCE`] of `now`.
///
/// # Errors
///
/// [`EventError`] otherwise.
pub(crate) fn verify_event(
	secret: &str,
	header: &str,
	body: &[u8],
	now: Timestamp,
) -> Result<Event, EventError> {
	let mut timestamp = None;
	let mut signatures = Vec::new();

	for part in header.split(',') {
		match part.trim().split_once('=') {
			Some(("t", value)) => timestamp = Some(value),
			Some(("v1", value)) => signatures.push(value),
			_ => {}
		}
	}

	let timestamp = timestamp.ok_or(EventError::Signature)?;
	let signed_at: i64 = timestamp.parse().map_err(|_| EventError::Signature)?;

	if now.timestamp().abs_diff(signed_at) > SIGNATURE_TOLERANCE.unsigned_abs() {
		return Err(EventError::Signature);
	}

	let expected = sign(secret, timestamp, body);
	// Every candidate is compared, so the time taken doesn't say which.
	let matched = signatures.iter().take(8).fold(false, |matched, candidate| {
		matched | bool::from(expected.as_bytes().ct_eq(candidate.as_bytes()))
	});

	if !matched {
		return Err(EventError::Signature);
	}

	let event: Value = serde_json::from_slice(body).map_err(|_| EventError::Malformed)?;
	let id = reference(&event["id"]).ok_or(EventError::Malformed)?;
	let kind = event["type"]
		.as_str()
		.filter(|kind| !kind.is_empty() && kind.len() <= 100)
		.ok_or(EventError::Malformed)?
		.to_owned();
	let object = event["data"]["object"].clone();

	if !object.is_object() {
		return Err(EventError::Malformed);
	}

	Ok(Event {
		id,
		kind,
		livemode: event["livemode"].as_bool().unwrap_or(false),
		object,
	})
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	const SECRET: &str = "whsec_test";

	fn at(text: &str) -> Timestamp {
		text.parse().expect("a time")
	}

	fn event() -> Vec<u8> {
		json!({
			"id": "evt_1",
			"type": "customer.subscription.updated",
			"data": { "object": { "id": "sub_1" } },
		})
		.to_string()
		.into_bytes()
	}

	#[test]
	fn an_event_is_taken_only_with_a_fresh_signature_of_its_bytes() {
		let now = at("2026-11-19T10:00:00Z");
		let body = event();
		let header = signature_header(SECRET, now.timestamp(), &body);
		let taken = verify_event(SECRET, &header, &body, now).expect("an event");
		assert_eq!(
			(taken.id.as_str(), taken.kind.as_str()),
			("evt_1", "customer.subscription.updated")
		);
		assert_eq!(taken.object["id"], "sub_1");
		assert!(!taken.livemode);
		// A second signature beside the right one, as when a secret is rolled.
		let rolled = format!("{header},v1={}", "0".repeat(64));
		assert!(verify_event(SECRET, &rolled, &body, now).is_ok());

		let mut other = body.clone();
		other.push(b' ');
		let late = now + chrono::TimeDelta::seconds(SIGNATURE_TOLERANCE + 1);
		let early = now - chrono::TimeDelta::seconds(SIGNATURE_TOLERANCE + 1);

		for (secret, header, body, now) in [
			("whsec_other", header.as_str(), &body, now),
			(SECRET, header.as_str(), &other, now),
			(SECRET, header.as_str(), &body, late),
			(SECRET, header.as_str(), &body, early),
			(SECRET, "", &body, now),
			(SECRET, "t=,v1=", &body, now),
			(SECRET, "v1=abc", &body, now),
			// A time nothing can be subtracted from.
			(SECRET, "t=-9223372036854775808,v1=abc", &body, now),
		] {
			assert_eq!(
				verify_event(secret, header, body, now),
				Err(EventError::Signature),
				"{header}"
			);
		}

		// The timestamp is part of what is signed.
		let moved = header.replace(
			&format!("t={}", now.timestamp()),
			&format!("t={}", now.timestamp() + 1),
		);
		assert_eq!(
			verify_event(SECRET, &moved, &body, now),
			Err(EventError::Signature)
		);
	}

	#[test]
	fn a_signed_body_that_is_not_an_event_is_malformed() {
		let now = at("2026-11-19T10:00:00Z");

		for body in [
			"not json".to_owned(),
			json!({ "type": "x", "data": { "object": {} } }).to_string(),
			json!({ "id": "evt/../1", "type": "x", "data": { "object": {} } }).to_string(),
			json!({ "id": "evt_1", "data": { "object": {} } }).to_string(),
			json!({ "id": "evt_1", "type": "x", "data": { "object": "sub_1" } }).to_string(),
		] {
			let header = signature_header(SECRET, now.timestamp(), body.as_bytes());
			assert_eq!(
				verify_event(SECRET, &header, body.as_bytes(), now),
				Err(EventError::Malformed),
				"{body}"
			);
		}
	}

	#[test]
	fn a_subscription_is_read_from_either_shape() {
		let current = json!({
			"id": "sub_1",
			"customer": "cus_1",
			"status": "active",
			"cancel_at_period_end": true,
			"items": { "data": [{
				"current_period_end": 1_800_000_000,
				"price": { "id": "price_pro" },
			}] },
		});
		let read = parse_subscription(&current).expect("a subscription");
		assert_eq!(read.status, SubscriptionStatus::Active);
		assert_eq!(read.price.as_deref(), Some("price_pro"));
		assert_eq!(
			read.period_end.map(|at| at.timestamp()),
			Some(1_800_000_000)
		);
		assert!(read.cancel_at_period_end);

		let older = json!({
			"id": "sub_1",
			"customer": { "id": "cus_1" },
			"status": "something_new",
			"current_period_end": 1_700_000_000,
			"items": { "data": [{ "price": "price_pro" }] },
		});
		let read = parse_subscription(&older).expect("a subscription");
		assert_eq!(read.customer, "cus_1");
		assert_eq!(read.status, SubscriptionStatus::Unknown);
		assert_eq!(
			read.period_end.map(|at| at.timestamp()),
			Some(1_700_000_000)
		);
		assert!(!read.cancel_at_period_end);

		assert_eq!(parse_subscription(&json!({ "id": "sub_1" })), None);
	}

	#[test]
	fn a_checkout_names_no_payment_method_types() {
		let checkout = Checkout {
			customer: "cus_1",
			price: "price_pro",
			organization: Uuid::nil(),
			success_url: "https://cloud.example/done",
			cancel_url: "https://cloud.example/back",
			automatic_tax: false,
		};
		let organization = checkout.organization.to_string();
		let form = checkout_form(&checkout, &organization);
		assert!(form.contains(&("mode", "subscription")));
		assert!(form.contains(&("line_items[0][price]", "price_pro")));
		assert!(
			form.iter()
				.all(|(name, _)| !name.starts_with("payment_method_types"))
		);
		assert!(
			form.iter()
				.all(|(name, _)| !name.starts_with("automatic_tax"))
		);
		let taxed = Checkout {
			automatic_tax: true,
			..checkout
		};
		assert!(checkout_form(&taxed, &organization).contains(&("automatic_tax[enabled]", "true")));
	}

	#[test]
	fn a_key_is_never_printed() {
		let stripe = Stripe::new("rk_test_abc").expect("a client");
		assert!(!format!("{stripe:?}").contains("rk_test_abc"));
		assert!(Stripe::new("pk_test_abc").is_err());
		assert!(Stripe::new("").is_err());
		assert!(format!("{stripe:?}").contains("api.stripe.com"));
		assert_eq!(stripe.secrets(), ["rk_test_abc"]);
		assert!(!Stripe::is_live_key("rk_test_abc"));
		assert!(Stripe::is_live_key("sk_live_abc"));
		assert!(!Stripe::is_live_key("rk_abc_live"));

		// The key only ever travels encrypted, or to this machine.
		for base in [
			"http://localhost:12111",
			"http://127.0.0.1:12111",
			"http://[::1]:1",
		] {
			assert!(Stripe::at(base, "rk_test_abc").is_ok(), "{base}");
		}

		for base in [
			"http://stripe.example",
			"ftp://localhost",
			"https://user:pass@stripe.example",
			"https://user@stripe.example",
			"https://stripe.example?x=1",
			"http://localhost.example",
		] {
			assert!(Stripe::at(base, "rk_test_abc").is_err(), "{base}");
		}
		assert!(is_id("sub_1AbC"));
		assert!(!is_id("sub/1"));
		assert!(!is_id(""));
	}

	#[tokio::test]
	async fn stripe_checkout_sends_api_version_and_handles_errors() {
		use std::sync::Arc;

		use leptoskit_axum::axum::Router;
		use leptoskit_axum::axum::extract::State;
		use leptoskit_axum::axum::http::HeaderMap;
		use leptoskit_axum::axum::http::StatusCode;
		use leptoskit_axum::axum::routing::post;
		use tokio::sync::Mutex;

		type RecordedRequests = Arc<Mutex<Vec<(String, String)>>>;

		let captured: RecordedRequests = Arc::new(Mutex::new(Vec::new()));
		let app = Router::new()
			.route(
				"/v1/checkout/sessions",
				post(
					|State(log): State<RecordedRequests>, headers: HeaderMap, body: String| {
						async move {
							let version = headers
								.get("stripe-version")
								.and_then(|v| v.to_str().ok())
								.unwrap_or("")
								.to_owned();
							log.lock().await.push((version, body));
							(
								StatusCode::OK,
								leptoskit_axum::axum::Json(json!({
									"id": "cs_test_1",
									"url": "https://checkout.stripe.com/c/pay/cs_test_1"
								})),
							)
						}
					},
				),
			)
			.route(
				"/v1/customers",
				post(|| {
					async {
						(
							StatusCode::BAD_REQUEST,
							leptoskit_axum::axum::Json(json!({
								"error": { "type": "invalid_request_error", "code": "parameter_invalid" }
							})),
						)
					}
				}),
			)
			.with_state(captured.clone());

		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		let server = tokio::spawn(async move {
			leptoskit_axum::axum::serve(listener, app).await.unwrap();
		});

		let stripe = Stripe::at(&format!("http://{addr}"), "rk_test_abc").unwrap();
		let url = stripe
			.checkout(Checkout {
				customer: "cus_123",
				price: "price_pro",
				organization: Uuid::nil(),
				success_url: "https://cloud.example/acme/settings/billing",
				cancel_url: "https://cloud.example/acme/settings/billing",
				automatic_tax: false,
			})
			.await
			.expect("checkout succeeds");
		assert_eq!(url, "https://checkout.stripe.com/c/pay/cs_test_1");

		let recorded = captured.lock().await;
		assert_eq!(recorded.len(), 1);
		assert_eq!(recorded[0].0, STRIPE_API_VERSION);
		assert!(recorded[0].1.contains("mode=subscription"));
		assert!(
			recorded[0]
				.1
				.contains("integration_identifier=leptoskit_cloud_plan_qhzrtwmk")
		);

		// Non-2xx Stripe JSON errors return Unavailable without panicking.
		let err = stripe
			.customer(NewCustomer {
				organization: Uuid::nil(),
			})
			.await
			.unwrap_err();
		assert!(matches!(err, PaymentError::Unavailable));

		server.abort();
	}
}
