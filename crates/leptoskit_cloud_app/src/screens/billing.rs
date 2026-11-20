//! `/<org>/settings/billing`: the plan, the spend cap, and this month's
//! usage (`docs/agents/cloud-design.md` §14.4).
//!
//! What it shows comes from `billing::subscription`, `plans` and `usage`;
//! a plan changes through `checkout` and the payment provider's portal
//! through `portal`, each a page at the provider that the console links to.
//! The design's chart and forecast need a usage forecast, and its invoices
//! need `invoices`; neither exists (§20), so neither is drawn.

use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit::query::MutateCallbacks;
use leptoskit_cloud::access::Permission;
use leptoskit_cloud::account::OrganizationSummary;
use leptoskit_cloud::billing::Checkout;
use leptoskit_cloud::billing::PlanList;
use leptoskit_cloud::billing::Portal;
use leptoskit_cloud::billing::SetSpendCap;
use leptoskit_cloud::billing::SubscriptionStatus;
use leptoskit_cloud::billing::SubscriptionSummary;
use leptoskit_cloud::billing::UsageBill;
use leptoskit_cloud::billing::UsageCost;
use leptoskit_cloud::billing::UsageReport;

use crate::components::Kind;
use crate::components::Tone;
use crate::components::field;
use crate::components::focus_ring;
use crate::components::link_button;
use crate::components::notice;
use crate::components::plain_button;
use crate::components::problem;
use crate::components::status_word;
use crate::console::Console;
use crate::console::Load;
use crate::console::rpc_problem;
use crate::data::plan_name;
use crate::routes::Navigation;
use crate::routes::OrgBilling;
use crate::screens::account::date;
use crate::shell::current_organization;
use crate::style;

/// Everything the billing page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BillingPage {
	/// The organisations haven't loaded.
	Loading,
	/// The person's organisations didn't load.
	Failed(String),
	/// No organisation by this slug that the person can see.
	NotFound {
		/// The slug in the address.
		slug: String,
	},
	/// The person's role doesn't see billing: only the plan's name.
	NotAllowed {
		/// The organisation.
		organization: OrganizationSummary,
	},
	/// The organisation's billing.
	Billing(Box<Billing>),
}

/// An organisation's billing, as an owner or billing member sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Billing {
	/// The organisation.
	pub organization: OrganizationSummary,
	/// Whether the person may change the plan and the cap.
	pub manage: bool,
	/// The plan and its subscription.
	pub subscription: Load<SubscriptionSummary>,
	/// The plans and prices.
	pub plans: Load<PlanList>,
	/// This month's usage.
	pub usage: Load<UsageReport>,
	/// The cap, as typed.
	pub cap: String,
	/// Whether the cap is being set.
	pub setting_cap: bool,
	/// Why setting the cap failed.
	pub cap_problem: Option<String>,
	/// A page at the payment provider, once asked for: checkout or portal.
	pub provider_link: Option<String>,
	/// Whether one is being asked for.
	pub opening: bool,
	/// Why asking failed.
	pub provider_problem: Option<String>,
}

impl BillingPage {
	/// The page the console's state calls for on `route`.
	#[must_use]
	pub fn read(console: &Console, route: &OrgBilling) -> Self {
		match console.me() {
			Load::Loading => Self::Loading,
			Load::Failed(error) => Self::Failed(error),
			Load::Ready(me) => {
				let current = crate::routes::Route::OrgBilling(route.clone());
				let Some(organization) = current_organization(&current, &me) else {
					return Self::NotFound {
						slug: route.org.clone(),
					};
				};

				if !organization.role.allows(Permission::ViewBilling) {
					return Self::NotAllowed {
						organization: organization.clone(),
					};
				}

				let billing = console.billing;
				// What was typed and asked for belongs to one organisation;
				// on any other, the page starts clean.
				let ours = billing.owner.get().as_deref() == Some(organization.slug.as_str());
				let link = billing
					.checkout
					.data()
					.or_else(|| billing.portal.data())
					.map(|link| link.url)
					.filter(|url| ours && url.starts_with("https://"));
				Self::Billing(Box::new(Billing {
					organization: organization.clone(),
					manage: organization.role.allows(Permission::ManageBilling),
					subscription: Load::of(&billing.subscription, rpc_problem),
					plans: Load::of(&billing.plans, rpc_problem),
					usage: Load::of(&billing.usage, rpc_problem),
					cap: if ours {
						billing.cap.get()
					} else {
						String::new()
					},
					setting_cap: ours && billing.set_cap.is_pending(),
					cap_problem: billing
						.set_cap
						.error()
						.filter(|_| ours)
						.map(|error| rpc_problem(&error)),
					provider_link: link,
					opening: ours && (billing.checkout.is_pending() || billing.portal.is_pending()),
					provider_problem: billing
						.checkout
						.error()
						.or_else(|| billing.portal.error())
						.filter(|_| ours)
						.map(|error| rpc_problem(&error)),
				}))
			}
		}
	}
}

/// `/<org>/settings/billing`.
pub fn page(route: &OrgBilling, _navigation: &Navigation) -> Element {
	let console = Console::get();
	let page = BillingPage::read(&console, route);

	if matches!(page, BillingPage::NotFound { .. }) {
		super::respond_with(404);
	}

	view(&console, &page)
}

/// The page.
pub fn view(console: &Console, page: &BillingPage) -> Element {
	let console = *console;
	let title = || Heading(1, "Billing").class(style::PAGE_TITLE);
	let body = match page {
		BillingPage::Loading => {
			vec![
				title(),
				super::loading("Loading billing", &["h-28 w-full", "h-48 w-full"]),
			]
		}
		BillingPage::Failed(error) => {
			let whoami = console.whoami;
			vec![
				title(),
				super::load_failed("your organisations", error, move || {
					drop(whoami.refetch());
				}),
			]
		}
		BillingPage::NotFound { slug } => {
			vec![
				Heading(1, "Not here.").class(style::PAGE_TITLE),
				Text(format!(
					"There's no organisation at {slug}, or you can't see it."
				))
				.class(style::SECONDARY),
				crate::components::link("Your organisations →", "/organizations"),
			]
		}
		BillingPage::NotAllowed { organization } => {
			vec![
				title(),
				View()
					.class(style::CARD)
					.child(Heading(2, "Plan").class(style::CARD_TITLE))
					.child(Text(plan_name(&organization.plan)).class(style::BODY))
					.child(
						Text("Billing is for owners and billing members.").class(style::SECONDARY),
					),
			]
		}
		BillingPage::Billing(billing) => {
			vec![
				title(),
				View()
					.class("flex-col lg:flex-row gap-4 w-full items-start")
					.child(
						View()
							.class("flex-col gap-4 w-full lg:w-[42%]")
							.child(plan(&console, billing))
							.child(cap(&console, billing)),
					)
					.child(
						View()
							.class("flex-col gap-4 w-full lg:flex-1")
							.child(receipt(&console, billing)),
					),
			]
		}
	};

	View()
		.class("flex-col gap-6 w-full")
		.test_id("billing")
		.children(body)
}

/// The plan, its price, and the way to change it.
fn plan(console: &Console, billing: &Billing) -> Element {
	let console = *console;
	let card = View()
		.class(style::CARD)
		.role(Role::Region)
		.accessibility_label("Plan")
		.child(Heading(2, "Plan").class(style::CARD_TITLE));

	let subscription = match &billing.subscription {
		Load::Loading => return card.child(super::loading("Loading the plan", &["h-11 w-full"])),
		Load::Failed(error) => {
			let query = console.billing.subscription;
			return card.child(super::load_failed("the plan", error, move || {
				drop(query.refetch());
			}));
		}
		Load::Ready(subscription) => subscription,
	};
	let plans = match &billing.plans {
		Load::Ready(plans) => Some(plans),
		_ => None,
	};
	let offered = plans.and_then(|plans| {
		plans
			.plans
			.iter()
			.find(|plan| plan.id == subscription.plan)
			.map(|plan| (plans.currency.clone(), plan))
	});
	let mut lines = vec![Text(plan_line(subscription, plans)).class(style::BODY)];

	if let Some((Some(currency), offered)) = &offered
		&& offered.included_usage > 0
	{
		lines.push(
			Text(format!(
				"{} of usage included",
				money(currency, offered.included_usage)
			))
			.class(style::SECONDARY),
		);
	}

	if let Some((tone, sentence)) = standing(subscription) {
		lines.push(status_word(tone, sentence));
	}

	let mut card = card.children(lines);

	if !subscription.payments {
		return card.child(Text("This cloud doesn't take payment.").class(style::MUTED));
	}

	if !billing.manage {
		return card.child(Text("Owners and billing members change the plan.").class(style::MUTED));
	}

	let organization = billing.organization.slug.clone();
	let busy = billing.opening.then_some("Opening…");
	let mut actions = Vec::new();

	if subscription
		.status
		.is_some_and(SubscriptionStatus::keeps_plan)
	{
		let portal = console.billing.portal;
		actions.push(focus_ring(
			console.focus,
			"billing-portal",
			plain_button(Kind::Secondary, "Payment and invoices", busy, move || {
				console.billing.checkout.reset();
				console.billing.owner.set(Some(organization.clone()));
				portal.mutate(Portal {
					organization: organization.clone(),
				});
			}),
		));
	} else {
		for sold in subscription
			.sold
			.iter()
			.filter(|plan| **plan != subscription.plan)
		{
			let checkout = console.billing.checkout;
			let organization = organization.clone();
			let plan = sold.clone();
			actions.push(focus_ring(
				console.focus,
				&format!("billing-checkout-{sold}"),
				plain_button(
					Kind::Secondary,
					format!("Move to {}", plan_name(sold)),
					busy,
					move || {
						console.billing.portal.reset();
						console.billing.owner.set(Some(organization.clone()));
						checkout.mutate(Checkout {
							organization: organization.clone(),
							plan: plan.clone(),
						});
					},
				),
			));
		}
	}

	card = card.child(
		View()
			.class("flex-row flex-wrap items-center gap-3")
			.children(actions),
	);

	if let Some(url) = &billing.provider_link {
		card = card
			.child(notice(
				"The payment provider's page is ready. It works once, for a short while.",
			))
			.child(focus_ring(
				console.focus,
				"billing-provider",
				link_button(Kind::Primary, "Continue to payment ↗", url.clone()),
			));
	}

	card.children(billing.provider_problem.clone().map(problem))
}

/// "Pro · $20.00 a month", or the name alone when nothing says its price.
fn plan_line(subscription: &SubscriptionSummary, plans: Option<&PlanList>) -> String {
	let name = plan_name(&subscription.plan);
	let Some(plans) = plans else {
		return name;
	};
	let Some(offered) = plans.plans.iter().find(|plan| plan.id == subscription.plan) else {
		return name;
	};

	match (&plans.currency, offered.monthly) {
		(Some(_), Some(0)) => format!("{} · free", offered.name),
		(Some(currency), Some(monthly)) => {
			format!("{} · {} a month", offered.name, money(currency, monthly))
		}
		(_, None) => format!("{} · priced by agreement", offered.name),
		(None, Some(_)) => offered.name.clone(),
	}
}

/// What the subscription's status means for the organisation, when it isn't
/// simply running.
fn standing(subscription: &SubscriptionSummary) -> Option<(Tone, String)> {
	let ends = subscription.period_end.as_ref().map(date);
	match subscription.status? {
		SubscriptionStatus::Active | SubscriptionStatus::Trialing
			if subscription.cancel_at_period_end =>
		{
			Some((
				Tone::Warn,
				ends.map_or_else(
					|| "Ends at the end of this period".to_owned(),
					|ends| format!("Ends on {ends}"),
				),
			))
		}
		SubscriptionStatus::Active => None,
		SubscriptionStatus::Trialing => {
			Some((
				Tone::Info,
				ends.map_or_else(
					|| "On trial".to_owned(),
					|ends| format!("On trial until {ends}"),
				),
			))
		}
		SubscriptionStatus::PastDue => {
			Some((
				Tone::Bad,
				"A payment didn't go through; the provider is trying again".to_owned(),
			))
		}
		SubscriptionStatus::Incomplete => {
			Some((Tone::Warn, "Waiting for the first payment".to_owned()))
		}
		SubscriptionStatus::IncompleteExpired
		| SubscriptionStatus::Canceled
		| SubscriptionStatus::Unpaid
		| SubscriptionStatus::Paused => {
			Some((
				Tone::Info,
				"The subscription is over; the organisation is on the plan everyone starts on"
					.to_owned(),
			))
		}
		SubscriptionStatus::Unknown => {
			Some((
				Tone::Warn,
				"The provider reports a status this console doesn't know".to_owned(),
			))
		}
	}
}

/// The spend cap, and the form that changes it.
fn cap(console: &Console, billing: &Billing) -> Element {
	let console = *console;
	let card = View()
		.class(style::CARD)
		.role(Role::Region)
		.accessibility_label("Spend cap")
		.child(Heading(2, "Spend cap").class(style::CARD_TITLE));
	let bill = match &billing.usage {
		Load::Loading => return card.child(super::loading("Loading the cap", &["h-11 w-full"])),
		Load::Failed(error) => {
			let query = console.billing.usage;
			return card.child(super::load_failed("the cap", error, move || {
				drop(query.refetch());
			}));
		}
		Load::Ready(UsageReport {
			cost: UsageCost::Unpriced { .. },
			..
		}) => {
			return card.child(
				Text("This cloud has no prices for the plan, so a cap would limit nothing.")
					.class(style::SECONDARY),
			);
		}
		Load::Ready(UsageReport {
			cost: UsageCost::Bill(bill),
			..
		}) => bill,
	};
	let mut card = card.child(Text(cap_line(bill)).class(style::BODY));

	if bill.cap_reached {
		card = card.child(status_word(
			Tone::Bad,
			"The cap is reached: new releases wait until next month or a higher cap",
		));
	}

	if !billing.manage {
		return card.child(Text("Owners and billing members change the cap.").class(style::MUTED));
	}

	let state = console.billing;
	let typing = console.typing;
	let input = TextInput()
		.value(billing.cap.clone())
		.placeholder(major(bill.spend_cap.unwrap_or(0), &bill.currency))
		.autocomplete("off")
		.on_change({
			let organization = billing.organization.slug.clone();
			move |text: &str| {
				if state.owner.get_untracked().as_deref() != Some(organization.as_str()) {
					state.set_cap.reset();
					state.checkout.reset();
					state.portal.reset();
					state.owner.set(Some(organization.clone()));
				}
				state.cap.set(text.to_owned());
			}
		})
		.on(NativeEventSlot::Focus, move |_| {
			typing.set(true);
			EventOutcome::Unhandled
		})
		.on(NativeEventSlot::Blur, move |_| {
			typing.set(false);
			EventOutcome::Unhandled
		});
	let typed = parse_major(&billing.cap, &bill.currency);
	let typed_problem = (!billing.cap.trim().is_empty() && typed.is_none()).then(|| {
		format!(
			"Type an amount in {}, such as 50.",
			bill.currency.to_uppercase()
		)
	});
	let preview = typed.map(|cap| format!("The cap will be {}.", money(&bill.currency, cap)));
	let organization = billing.organization.slug.clone();
	let busy = billing.setting_cap.then_some("Saving…");
	let mut set = plain_button(Kind::Secondary, "Set cap", busy, {
		let organization = organization.clone();
		move || {
			if let Some(cap) = typed {
				state.owner.set(Some(organization.clone()));
				state.set_cap.mutate_with(
					SetSpendCap {
						organization: organization.clone(),
						cap: Some(cap),
					},
					MutateCallbacks::new().on_success(move |_| state.cap.set(String::new())),
				);
			}
		}
	});

	if typed.is_none() {
		set = set.enabled(false);
	}

	let mut actions = vec![focus_ring(console.focus, "billing-cap-set", set)];

	if bill.own_spend_cap {
		actions.push(focus_ring(
			console.focus,
			"billing-cap-plan",
			plain_button(Kind::Quiet, "Use the plan's cap", busy, move || {
				state.owner.set(Some(organization.clone()));
				state.set_cap.mutate(SetSpendCap {
					organization: organization.clone(),
					cap: None,
				});
			}),
		));
	}

	card.child(field(
		"billing-cap",
		&format!("New cap, in {}", bill.currency.to_uppercase()),
		input,
		typed_problem.as_deref(),
	))
	.children(preview.map(|preview| Text(preview).class(style::SECONDARY)))
	.child(
		View()
			.class("flex-row flex-wrap items-center gap-3")
			.children(actions),
	)
	.children(billing.cap_problem.clone().map(problem))
}

/// "$50.00 of usage a month beyond the plan, your own cap", or that there is
/// none.
fn cap_line(bill: &UsageBill) -> String {
	match bill.spend_cap {
		None => "No cap: usage beyond the plan is billed as it comes.".to_owned(),
		Some(cap) => {
			format!(
				"{} of usage a month beyond the plan{}.",
				money(&bill.currency, cap),
				if bill.own_spend_cap {
					", a cap you set"
				} else {
					", the plan's cap"
				}
			)
		}
	}
}

/// This month's usage as a receipt: one line per meter, then the totals.
fn receipt(console: &Console, billing: &Billing) -> Element {
	let console = *console;
	let card = View()
		.class(style::CARD)
		.role(Role::Region)
		.accessibility_label("Usage");
	let report = match &billing.usage {
		Load::Loading => {
			return card
				.child(Heading(2, "This month so far").class(style::CARD_TITLE))
				.child(super::loading("Loading usage", &["h-48 w-full"]));
		}
		Load::Failed(error) => {
			let query = console.billing.usage;
			return card
				.child(Heading(2, "This month so far").class(style::CARD_TITLE))
				.child(super::load_failed("this month's usage", error, move || {
					drop(query.refetch());
				}));
		}
		Load::Ready(report) => report,
	};
	let card = card
		.child(Heading(2, format!("{} so far", month(&report.period))).class(style::CARD_TITLE));

	match &report.cost {
		UsageCost::Unpriced { reason } => {
			card.child(Text(format!("Usage isn't priced here: {reason}")).class(style::SECONDARY))
				.child(rows(
					report
						.quantities
						.iter()
						.map(|quantity| {
							(
								quantity.meter.clone(),
								count(quantity.quantity),
								String::new(),
							)
						})
						.collect(),
				))
		}
		UsageCost::Bill(bill) => {
			let currency = &bill.currency;
			let mut lines: Vec<(String, String, String)> = bill
				.lines
				.iter()
				.map(|line| {
					let quantity = if line.allowance > 0 {
						format!("{} of {} free", count(line.quantity), count(line.allowance))
					} else {
						count(line.quantity)
					};
					(line.meter.clone(), quantity, money(currency, line.amount))
				})
				.collect();
			lines.push((
				"Usage".to_owned(),
				String::new(),
				money(currency, bill.usage),
			));

			if bill.included > 0 {
				lines.push((
					"Included in the plan".to_owned(),
					String::new(),
					format!("−{}", money(currency, bill.included)),
				));
			}

			if let Some(plan) = bill.plan_price {
				lines.push(("Plan".to_owned(), String::new(), money(currency, plan)));
			}

			lines.push((
				"So far".to_owned(),
				String::new(),
				money(currency, bill.total),
			));
			let mut card = card.child(rows(lines));

			if !bill.unpriced.is_empty() {
				card = card.child(
					Text(format!(
						"No price yet for {}: counted, and not charged.",
						bill.unpriced.join(", ")
					))
					.class(style::MUTED),
				);
			}

			card
		}
	}
}

/// A table of label, quantity and amount.
fn rows(lines: Vec<(String, String, String)>) -> Element {
	View()
		.class("flex-col w-full")
		.role(Role::List)
		.accessibility_label("Usage by meter")
		.children(lines.into_iter().map(|(label, quantity, amount)| {
			View()
				.role(Role::ListItem)
				.class(
					"flex-row flex-wrap items-center gap-3 py-2 border-b \
					 border-[var(--cloud-line)]",
				)
				.child(Text(label).class(style::BODY).class("flex-1"))
				.children((!quantity.is_empty()).then(|| Text(quantity).class(style::SECONDARY)))
				.child(Text(amount).class(style::CODE).class("min-w-20"))
		}))
}

/// `2026-11` as "November 2026".
fn month(period: &str) -> String {
	const MONTHS: [&str; 12] = [
		"January",
		"February",
		"March",
		"April",
		"May",
		"June",
		"July",
		"August",
		"September",
		"October",
		"November",
		"December",
	];
	period
		.split_once('-')
		.and_then(|(year, month)| {
			let index = month.parse::<usize>().ok()?.checked_sub(1)?;
			Some(format!("{} {year}", MONTHS.get(index)?))
		})
		.unwrap_or_else(|| period.to_owned())
}

/// A whole number with thousands separated: `1,140`.
fn count(value: u64) -> String {
	let digits = value.to_string();
	let mut out = String::new();

	for (index, digit) in digits.chars().enumerate() {
		if index > 0 && (digits.len() - index) % 3 == 0 {
			out.push(',');
		}
		out.push(digit);
	}

	out
}

/// How many decimal places `currency`'s minor unit has, as the payment
/// provider counts them.
fn decimals(currency: &str) -> u32 {
	match currency.to_ascii_lowercase().as_str() {
		"bif" | "clp" | "djf" | "gnf" | "isk" | "jpy" | "kmf" | "krw" | "mga" | "pyg" | "rwf"
		| "ugx" | "vnd" | "vuv" | "xaf" | "xof" | "xpf" => 0,
		"bhd" | "iqd" | "jod" | "kwd" | "lyd" | "omr" | "tnd" => 3,
		_ => 2,
	}
}

/// How many minor units a major unit of `currency` has.
fn scale(currency: &str) -> u64 {
	10_u64.pow(decimals(currency))
}

/// The part after the point, padded to `currency`'s decimal places.
fn fraction(minor: u64, currency: &str) -> String {
	let places = decimals(currency) as usize;
	format!("{:0places$}", minor % scale(currency))
}

/// An amount in minor units, as people write it: `$20.00`, `€9.12`,
/// `1,200 JPY`.
#[must_use]
pub fn money(currency: &str, minor: u64) -> String {
	let scale = scale(currency);
	let whole = count(minor / scale);
	let amount = if scale == 1 {
		whole
	} else {
		format!("{whole}.{}", fraction(minor, currency))
	};

	match currency.to_ascii_lowercase().as_str() {
		"usd" => format!("${amount}"),
		"eur" => format!("€{amount}"),
		"gbp" => format!("£{amount}"),
		other => format!("{amount} {}", other.to_uppercase()),
	}
}

/// An amount in minor units, in major units with no symbol, for a field:
/// `50` or `12.50`.
fn major(minor: u64, currency: &str) -> String {
	let scale = scale(currency);
	if minor.is_multiple_of(scale) {
		(minor / scale).to_string()
	} else {
		format!("{}.{}", minor / scale, fraction(minor, currency))
	}
}

/// What was typed, in major units, as minor units: `50` is 5000 cents.
///
/// A comma counts only as a thousands separator, between groups of three
/// digits before the point: `50,00`, meaning fifty in much of Europe, is
/// refused rather than read as five thousand.
#[must_use]
pub fn parse_major(text: &str, currency: &str) -> Option<u64> {
	let text = text.trim();
	let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
	let mut groups = whole.split(',');
	let first = groups.next()?;
	let rest: Vec<&str> = groups.collect();

	if first.is_empty()
		|| (!rest.is_empty() && first.len() > 3)
		|| rest.iter().any(|group| group.len() != 3)
	{
		return None;
	}

	let digits: String = std::iter::once(first).chain(rest).collect();

	if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
		return None;
	}

	let places = decimals(currency) as usize;

	if fraction.len() > places
		|| (text.contains('.') && fraction.is_empty())
		|| !fraction.bytes().all(|byte| byte.is_ascii_digit())
	{
		return None;
	}

	let padded = format!("{fraction:0<places$}");
	let fraction: u64 = if padded.is_empty() {
		0
	} else {
		padded.parse().ok()?
	};

	digits
		.parse::<u64>()
		.ok()?
		.checked_mul(scale(currency))?
		.checked_add(fraction)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn amounts_read_as_people_write_them() {
		assert_eq!(money("usd", 2000), "$20.00");
		assert_eq!(money("usd", 912), "$9.12");
		assert_eq!(money("eur", 123_456), "€1,234.56");
		assert_eq!(money("jpy", 1200), "1,200 JPY");
		assert_eq!(money("chf", 5), "0.05 CHF");
		assert_eq!(money("kwd", 1234), "1.234 KWD");
		assert_eq!(money("pyg", 50000), "50,000 PYG");
	}

	#[test]
	fn a_typed_cap_is_read_in_major_units() {
		assert_eq!(parse_major("50", "usd"), Some(5000));
		assert_eq!(parse_major(" 12.5 ", "usd"), Some(1250));
		assert_eq!(parse_major("1,000.05", "usd"), Some(100_005));
		assert_eq!(parse_major("12,345,678", "usd"), Some(1_234_567_800));
		// A decimal comma is refused, never read as thousands.
		assert_eq!(parse_major("50,00", "eur"), None);
		assert_eq!(parse_major("12,5", "eur"), None);
		assert_eq!(parse_major("1,0,0", "usd"), None);
		assert_eq!(parse_major("1000,000", "usd"), None);
		assert_eq!(parse_major("$50", "usd"), None);
		assert_eq!(parse_major("50.", "usd"), None);
		assert_eq!(parse_major("1.234", "kwd"), Some(1234));
		assert_eq!(parse_major("50000", "pyg"), Some(50000));
		assert_eq!(parse_major("1200", "jpy"), Some(1200));
		assert_eq!(parse_major("12.5", "jpy"), None);
		assert_eq!(parse_major("12.345", "usd"), None);
		assert_eq!(parse_major("-5", "usd"), None);
		assert_eq!(parse_major("", "usd"), None);
		assert_eq!(parse_major("ten", "usd"), None);
		assert_eq!(major(5000, "usd"), "50");
		assert_eq!(major(1250, "usd"), "12.50");
		assert_eq!(major(1005, "kwd"), "1.005");
	}

	#[test]
	fn periods_and_counts_read_as_words() {
		assert_eq!(month("2026-11"), "November 2026");
		assert_eq!(month("2026-13"), "2026-13");
		assert_eq!(count(1_140), "1,140");
		assert_eq!(count(12), "12");
		assert_eq!(count(1_000_000), "1,000,000");
	}
}
