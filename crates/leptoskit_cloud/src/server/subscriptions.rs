//! The bodies of [`crate::billing`]'s `subscription`, `checkout` and
//! `portal`, and what the payment provider's events do.
//!
//! An organisation's plan is changed in one place: [`apply`], from what
//! the provider says a subscription is now. `checkout` and `portal` only
//! hand out a page at the provider; leaving that page changes nothing
//! here. Each call to the provider is made outside any transaction, after
//! a query or a mutation has checked who is asking.

use std::future::Future;

use leptoskit_axum::axum::body::Body;
use leptoskit_axum::axum::extract::State;
use leptoskit_axum::axum::http::HeaderMap;
use leptoskit_axum::axum::http::StatusCode;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::Services;
use super::audit;
use super::audit::Actor;
use super::authorize;
use super::identity::client;
use super::jobs;
use super::payments;
use super::payments::Checkout;
use super::payments::Event;
use super::payments::EventError;
use super::payments::NewCustomer;
use super::payments::PaymentError;
use super::payments::PaymentsConfig;
use super::payments::Subscription;
use super::personal::STARTING_PLAN;
use super::services;
use crate::access::Permission;
use crate::billing::BillingLink;
use crate::billing::SubscriptionStatus;
use crate::billing::SubscriptionSummary;
use crate::tables::BillingAccount;
use crate::tables::NewBillingAccount;
use crate::tables::NewPaymentEvent;
use crate::tables::Organization;
use crate::tables::PaymentEvent;

/// Where the payment provider sends its events.
pub const STRIPE_WEBHOOK_PATH: &str = "/webhooks/stripe";

fn unavailable() -> rpc::Error {
	rpc::Error::conflict("payments_unavailable: this cloud doesn't take payment")
}

fn unreachable_provider(_: PaymentError) -> rpc::Error {
	rpc::Error::conflict(
		"payments_unavailable: the payment provider couldn't be reached just now; try again",
	)
}

async fn account(
	db: &impl leptoskit_db::Read,
	organization: Id<Organization>,
) -> leptoskit_db::Result<Option<BillingAccount>> {
	BillingAccount::query()
		.filter(|row| row.organization_id.eq(organization))
		.first(db)
		.await
}

fn status(account: &BillingAccount) -> Option<SubscriptionStatus> {
	account.status.as_deref().map(SubscriptionStatus::parse)
}

/// The plans `config` has a price for and the catalogue sells for money.
fn sold(services: &Services, config: &PaymentsConfig) -> Vec<String> {
	let Some(catalogue) = &services.shipping().catalogue else {
		return Vec::new();
	};
	catalogue
		.plans()
		.iter()
		.filter(|plan| !plan.is_free() && config.prices.contains_key(plan.id.as_str()))
		.map(|plan| plan.id.as_str().to_owned())
		.collect()
}

pub(crate) async fn subscription(
	cx: &rpc::QueryCx,
	organization: &str,
) -> rpc::Result<SubscriptionSummary> {
	let access =
		authorize::organization(cx.db(), cx.context(), organization, Permission::ViewBilling)
			.await?;
	let services = services(cx.context())?;
	let config = services.shipping().payments.as_ref();
	let account = account(cx.db(), access.organization.id).await?;
	Ok(SubscriptionSummary {
		plan: access.organization.plan.clone(),
		payments: config.is_some(),
		sold: config.map_or_else(Vec::new, |config| sold(&services, config)),
		status: account.as_ref().and_then(status),
		period_end: account.as_ref().and_then(|account| account.period_end),
		cancel_at_period_end: account
			.as_ref()
			.and_then(|account| account.cancel_at_period_end)
			.unwrap_or(false),
	})
}

/// What `checkout` needs to know before it asks the provider, checked and
/// kept as the caller: who may not manage the bill never causes a call.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct StartCheckout {
	organization: String,
	plan: String,
	/// The customer the provider just made, when the organisation had none.
	made: Option<String>,
}

/// What [`StartCheckout`] answers.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Started {
	organization: Uuid,
	slug: String,
	name: String,
	/// The organisation's customer, or `None` when one must be made first.
	customer: Option<String>,
	price: String,
}

impl rpc::Operation for StartCheckout {
	type Context = rpc::MutationCx;
	type Output = Started;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	const NAME: &'static str = "checkout";

	fn run(self, cx: &rpc::MutationCx) -> impl Future<Output = rpc::Result<Started>> + Send {
		start_checkout(cx, self)
	}
}

async fn start_checkout(cx: &rpc::MutationCx, args: StartCheckout) -> rpc::Result<Started> {
	let access = authorize::organization(
		cx.tx(),
		cx.context(),
		&args.organization,
		Permission::ManageBilling,
	)
	.await?;
	let services = services(cx.context())?;
	let Some(config) = &services.shipping().payments else {
		return Err(unavailable());
	};
	let plan = services
		.shipping()
		.catalogue
		.as_ref()
		.and_then(|catalogue| catalogue.plan(&args.plan))
		.ok_or_else(|| {
			rpc::Error::not_found(format!(
				"plan_not_found: this cloud has no `{}` plan",
				args.plan
			))
		})?;
	let price = config
		.prices
		.get(plan.id.as_str())
		.filter(|_| !plan.is_free())
		.ok_or_else(|| {
			rpc::Error::conflict(format!(
				"plan_not_sold: the `{}` plan can't be subscribed to here",
				plan.id
			))
		})?
		.clone();
	let organization_id = access.organization.id;
	let existing = account(cx.tx(), organization_id).await?;

	if existing
		.as_ref()
		.is_some_and(|account| status(account).is_some_and(SubscriptionStatus::keeps_plan))
	{
		return Err(rpc::Error::conflict(format!(
			"already_subscribed: `{}` has a subscription; change or cancel it with `portal`",
			access.organization.slug
		)));
	}

	let made = args.made.filter(|customer| payments::is_id(customer));
	// The first customer kept is the organisation's: a second checkout
	// racing the first uses it, and never a second customer.
	let customer = match existing {
		Some(account) if account.customer.is_some() => account.customer,
		Some(mut account) => {
			if made.is_some() {
				account.customer.clone_from(&made);
				account.save(cx.tx()).await?;
			}

			made
		}
		None => {
			if made.is_some() {
				NewBillingAccount {
					organization_id,
					spend_cap: None,
					customer: made.clone(),
					subscription: None,
					status: None,
					period_end: None,
					cancel_at_period_end: None,
				}
				.insert(cx.tx())
				.await?;
			}

			made
		}
	};

	if customer.is_some() {
		audit::record(
			cx.tx(),
			&Actor::from(&access.caller),
			"billing.checkout",
			Some(organization_id),
			None,
			Some(plan.id.as_str().to_owned()),
			&client(cx.context()),
		)
		.await?;
	}

	Ok(Started {
		organization: organization_id.uuid(),
		slug: access.organization.slug.clone(),
		name: access.organization.name.clone(),
		customer,
		price,
	})
}

fn return_url(services: &Services, slug: &str, outcome: Option<&str>) -> String {
	let page = format!("{}/{slug}/settings/billing", services.origin());

	match outcome {
		Some(outcome) => format!("{page}?checkout={outcome}"),
		None => page,
	}
}

/// `checkout`'s body: who may, then the customer, then the page.
pub(crate) async fn checkout(
	cx: &rpc::ActionCx,
	organization: String,
	plan: String,
) -> rpc::Result<BillingLink> {
	let services = services(cx.context())?;
	let mut started = cx
		.mutate(StartCheckout {
			organization: organization.clone(),
			plan: plan.clone(),
			made: None,
		})
		.await?;
	let Some(config) = &services.shipping().payments else {
		return Err(unavailable());
	};

	if started.customer.is_none() {
		let made = config
			.provider
			.customer(NewCustomer {
				organization: started.organization,
				slug: &started.slug,
				name: &started.name,
			})
			.await
			.map_err(unreachable_provider)?;
		started = cx
			.mutate(StartCheckout {
				organization,
				plan,
				made: Some(made),
			})
			.await?;
	}

	let Some(customer) = &started.customer else {
		return Err(unreachable_provider(PaymentError::Unavailable));
	};
	let url = config
		.provider
		.checkout(Checkout {
			customer,
			price: &started.price,
			organization: started.organization,
			success_url: &return_url(&services, &started.slug, Some("done")),
			cancel_url: &return_url(&services, &started.slug, Some("cancelled")),
			automatic_tax: config.automatic_tax,
		})
		.await
		.map_err(unreachable_provider)?;
	Ok(BillingLink { url })
}

/// What `portal` needs to know before it asks the provider.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct OpenPortal {
	organization: String,
}

impl rpc::Operation for OpenPortal {
	type Context = rpc::MutationCx;
	type Output = (String, String);

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	const NAME: &'static str = "portal";

	fn run(
		self,
		cx: &rpc::MutationCx,
	) -> impl Future<Output = rpc::Result<(String, String)>> + Send {
		open_portal(cx, self)
	}
}

/// The organisation's slug and customer, as the caller.
async fn open_portal(cx: &rpc::MutationCx, args: OpenPortal) -> rpc::Result<(String, String)> {
	let access = authorize::organization(
		cx.tx(),
		cx.context(),
		&args.organization,
		Permission::ManageBilling,
	)
	.await?;

	if services(cx.context())?.shipping().payments.is_none() {
		return Err(unavailable());
	}

	let organization_id = access.organization.id;
	let customer = account(cx.tx(), organization_id)
		.await?
		.and_then(|account| account.customer)
		.ok_or_else(|| {
			rpc::Error::conflict(format!(
				"no_customer: `{}` has never started a subscription; start one with `checkout`",
				access.organization.slug
			))
		})?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"billing.portal",
		Some(organization_id),
		None,
		None,
		&client(cx.context()),
	)
	.await?;
	Ok((access.organization.slug.clone(), customer))
}

/// `portal`'s body: who may, then the page.
pub(crate) async fn portal(cx: &rpc::ActionCx, organization: String) -> rpc::Result<BillingLink> {
	let services = services(cx.context())?;
	let (slug, customer) = cx.mutate(OpenPortal { organization }).await?;
	let Some(config) = &services.shipping().payments else {
		return Err(unavailable());
	};
	let url = config
		.provider
		.portal(&customer, &return_url(&services, &slug, None))
		.await
		.map_err(unreachable_provider)?;
	Ok(BillingLink { url })
}

/// What an event is a reason to do.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reason {
	/// Ask what the subscription is now, and follow it.
	Subscription(String),
	/// Note, for the organisation's audit log, that money came back out.
	Risk {
		action: &'static str,
		/// The customer, when the event's object names one.
		customer: Option<String>,
		/// The charge to ask the provider about, when it doesn't.
		charge: Option<String>,
	},
	/// Nothing.
	Nothing,
}

/// Why `event` matters, from its kind and the IDs in its object. Nothing
/// else in the object is believed.
fn reason(event: &Event) -> Reason {
	let object = &event.object;
	let subscription = |id: Option<String>| id.map_or(Reason::Nothing, Reason::Subscription);

	match event.kind.as_str() {
		"customer.subscription.created"
		| "customer.subscription.updated"
		| "customer.subscription.deleted"
		| "customer.subscription.paused"
		| "customer.subscription.resumed" => subscription(payments::reference(&object["id"])),
		"checkout.session.completed" | "checkout.session.async_payment_succeeded" => {
			subscription(payments::reference(&object["subscription"]))
		}
		"invoice.paid" | "invoice.payment_failed" => {
			subscription(
				payments::reference(&object["parent"]["subscription_details"]["subscription"])
					.or_else(|| payments::reference(&object["subscription"])),
			)
		}
		"charge.refunded" => {
			Reason::Risk {
				action: "billing.refund",
				customer: payments::reference(&object["customer"]),
				charge: None,
			}
		}
		"charge.dispute.created" => {
			Reason::Risk {
				action: "billing.dispute",
				customer: None,
				charge: payments::reference(&object["charge"]),
			}
		}
		"radar.early_fraud_warning.created" => {
			Reason::Risk {
				action: "billing.fraud_warning",
				customer: None,
				charge: payments::reference(&object["charge"]),
			}
		}
		_ => Reason::Nothing,
	}
}

/// What to write once the provider has been asked.
#[derive(Debug, Clone)]
enum Write {
	Follow(Subscription),
	Note {
		action: &'static str,
		customer: String,
	},
	Nothing,
}

/// The plan an organisation is on while `subscription` is what the
/// provider has: `None` leaves the plan as it is.
fn plan_for(
	services: &Services,
	config: &PaymentsConfig,
	subscription: &Subscription,
) -> Option<String> {
	if subscription.status.is_over() {
		return Some(STARTING_PLAN.to_owned());
	}

	if !subscription.status.keeps_plan() {
		return None;
	}

	let plan = config.plan_of(subscription.price.as_deref()?)?;
	// A price for a plan the catalogue doesn't sell for money changes
	// nothing: the organisation would be unpriced, and so uncapped.
	services
		.shipping()
		.catalogue
		.as_ref()
		.and_then(|catalogue| catalogue.plan(plan))
		.filter(|plan| !plan.is_free())
		.map(|plan| plan.id.as_str().to_owned())
}

/// Makes the organisation whose customer `subscription` belongs to follow
/// it, and answers that organisation. A subscription that is over never
/// replaces another the organisation follows: an old one ending says
/// nothing about the one that replaced it.
async fn apply(
	tx: &leptoskit_db::Tx,
	services: &Services,
	config: &PaymentsConfig,
	subscription: &Subscription,
) -> rpc::Result<Option<Id<Organization>>> {
	let customer = subscription.customer.as_str();
	let Some(mut account) = BillingAccount::query()
		.filter(|row| row.customer.eq(customer))
		.first(tx)
		.await?
	else {
		return Ok(None);
	};
	let organization_id = account.organization_id;
	let follows_another = account
		.subscription
		.as_deref()
		.is_some_and(|current| current != subscription.id);

	if follows_another && !subscription.status.keeps_plan() {
		return Ok(Some(organization_id));
	}

	account.subscription = Some(subscription.id.clone());
	account.status = Some(subscription.status.code().to_owned());
	account.period_end = subscription.period_end;
	account.cancel_at_period_end = Some(subscription.cancel_at_period_end);
	account.save(tx).await?;
	let mut organization = Organization::query()
		.filter(|row| row.id.eq(organization_id))
		.one(tx)
		.await?;

	match plan_for(services, config, subscription) {
		Some(plan) if plan != organization.plan => {
			let target = format!("{} to {plan}", organization.plan);
			organization.plan = plan;
			organization.save(tx).await?;
			audit::record_provider(tx, "billing.plan_changed", organization_id, Some(target))
				.await?;
		}
		Some(_) => {}
		None if subscription.status.keeps_plan() => {
			audit::record_provider(
				tx,
				"billing.unknown_price",
				organization_id,
				subscription.price.clone(),
			)
			.await?;
		}
		None => {}
	}

	Ok(Some(organization_id))
}

/// Writes what an event came to, and that the event is dealt with, in one
/// transaction.
async fn settle(services: &Services, event: &Event, write: &Write) -> rpc::Result<()> {
	let owned = (services.clone(), event.clone(), write.clone());
	jobs::serializable(services.db(), move |tx| {
		let (services, event, write) = owned.clone();
		Box::pin(async move {
			let Some(config) = &services.shipping().payments else {
				return Ok(());
			};
			let organization = match &write {
				Write::Follow(subscription) => apply(tx, &services, config, subscription).await?,
				Write::Note { action, customer } => {
					let customer = customer.as_str();
					let account = BillingAccount::query()
						.filter(|row| row.customer.eq(customer))
						.first(tx)
						.await?;

					if let Some(account) = &account {
						audit::record_provider(
							tx,
							action,
							account.organization_id,
							Some(event.id.clone()),
						)
						.await?;
					}

					account.map(|account| account.organization_id)
				}
				Write::Nothing => None,
			};
			NewPaymentEvent {
				event: event.id.clone(),
				kind: event.kind.clone(),
				organization_id: organization,
			}
			.insert(tx)
			.await?;
			Ok(())
		})
	})
	.await
}

async fn seen(services: &Services, event: &str) -> leptoskit_db::Result<bool> {
	PaymentEvent::query()
		.filter(|row| row.event.eq(event))
		.first(services.db())
		.await
		.map(|row| row.is_some())
}

/// Deals with one event: `Ok` once it is, and `Err` when the provider
/// should send it again.
async fn deal_with(services: &Services, config: &PaymentsConfig, event: &Event) -> Result<(), ()> {
	if seen(services, &event.id).await.map_err(|_| ())? {
		return Ok(());
	}

	let write = match reason(event) {
		Reason::Subscription(id) => {
			match config.provider.subscription(&id).await {
				Ok(subscription) => Write::Follow(subscription),
				// A subscription the provider doesn't have is nobody's.
				Err(PaymentError::NotFound) => Write::Nothing,
				Err(PaymentError::Unavailable) => return Err(()),
			}
		}
		Reason::Risk {
			action,
			customer,
			charge,
		} => {
			let customer = match (customer, charge) {
				(Some(customer), _) => Some(customer),
				(None, Some(charge)) => {
					match config.provider.charge_customer(&charge).await {
						Ok(customer) => customer,
						Err(PaymentError::NotFound) => None,
						Err(PaymentError::Unavailable) => return Err(()),
					}
				}
				(None, None) => None,
			};
			customer.map_or(Write::Nothing, |customer| Write::Note { action, customer })
		}
		Reason::Nothing => Write::Nothing,
	};

	match settle(services, event, &write).await {
		Ok(()) => Ok(()),
		// The same event, dealt with by another delivery meanwhile.
		Err(rpc::Error::Conflict(_)) if seen(services, &event.id).await.unwrap_or(false) => Ok(()),
		Err(_) => Err(()),
	}
}

/// The route the provider posts its events to. It answers 200 for an
/// event that is dealt with, whatever it was; 400 for a body that isn't a
/// signed event; and 503 when the event should be sent again.
pub(crate) async fn stripe_webhook(
	State(services): State<Services>,
	headers: HeaderMap,
	body: Body,
) -> StatusCode {
	let Some(config) = &services.shipping().payments else {
		return StatusCode::NOT_FOUND;
	};
	let Ok(body) = leptoskit_axum::axum::body::to_bytes(body, payments::MAX_EVENT_LEN).await else {
		return StatusCode::PAYLOAD_TOO_LARGE;
	};
	let signature = headers
		.get("stripe-signature")
		.and_then(|value| value.to_str().ok())
		.unwrap_or_default();
	let event = match payments::verify_event(
		&config.webhook_secret,
		signature,
		&body,
		leptoskit_db::now(),
	) {
		Ok(event) => event,
		Err(EventError::Signature | EventError::Malformed) => return StatusCode::BAD_REQUEST,
	};

	match deal_with(&services, config, &event).await {
		Ok(()) => StatusCode::OK,
		Err(()) => StatusCode::SERVICE_UNAVAILABLE,
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	fn event(kind: &str, object: serde_json::Value) -> Event {
		Event {
			id: "evt_1".to_owned(),
			kind: kind.to_owned(),
			object,
		}
	}

	#[test]
	fn an_event_is_only_a_reason_to_ask() {
		let follow = |id: &str| Reason::Subscription(id.to_owned());
		assert_eq!(
			reason(&event(
				"customer.subscription.deleted",
				json!({ "id": "sub_1", "status": "active" })
			)),
			follow("sub_1")
		);
		assert_eq!(
			reason(&event(
				"checkout.session.completed",
				json!({ "id": "cs_1", "subscription": "sub_2" })
			)),
			follow("sub_2")
		);
		// A checkout that wasn't for a subscription is nothing here.
		assert_eq!(
			reason(&event(
				"checkout.session.completed",
				json!({ "id": "cs_1", "subscription": null })
			)),
			Reason::Nothing
		);
		assert_eq!(
			reason(&event(
				"invoice.payment_failed",
				json!({ "parent": { "subscription_details": { "subscription": "sub_3" } } })
			)),
			follow("sub_3")
		);
		assert_eq!(
			reason(&event("invoice.paid", json!({ "subscription": "sub_4" }))),
			follow("sub_4")
		);
		assert_eq!(
			reason(&event(
				"charge.dispute.created",
				json!({ "id": "dp_1", "charge": "ch_1" })
			)),
			Reason::Risk {
				action: "billing.dispute",
				customer: None,
				charge: Some("ch_1".to_owned()),
			}
		);
		assert_eq!(
			reason(&event(
				"charge.refunded",
				json!({ "id": "ch_1", "customer": "cus_1" })
			)),
			Reason::Risk {
				action: "billing.refund",
				customer: Some("cus_1".to_owned()),
				charge: None,
			}
		);
		assert_eq!(
			reason(&event("customer.created", json!({ "id": "cus_1" }))),
			Reason::Nothing
		);
		// An ID that isn't one is never asked about.
		assert_eq!(
			reason(&event(
				"customer.subscription.updated",
				json!({ "id": "../v1/customers" })
			)),
			Reason::Nothing
		);
	}
}
