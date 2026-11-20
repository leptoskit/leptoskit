//! The bodies of [`crate::billing`]'s operations, the spend cap, and the
//! plans' limits.
//!
//! Everything here reads the operator's price catalogue. A control plane
//! without one, or an organisation whose plan the catalogue doesn't have,
//! is *unpriced*: usage is still counted and shown, and nothing is capped
//! or limited, because nothing says what the cap or the limit would be.

use leptoskit_auth::Account;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;

use super::Shipping;
use super::audit;
use super::audit::Actor;
use super::authorize;
use super::identity::caller;
use super::identity::client;
use super::metering;
use super::services;
use crate::access::Permission;
use crate::billing::MeterPrice;
use crate::billing::MeterQuantity;
use crate::billing::PlanLimits;
use crate::billing::PlanList;
use crate::billing::PlanSummary;
use crate::billing::ProjectUsage;
use crate::billing::SpendCap;
use crate::billing::UsageBill;
use crate::billing::UsageCost;
use crate::billing::UsageReport;
use crate::deployments::CostLine;
use crate::pricing::Catalogue;
use crate::pricing::EstimateInput;
use crate::pricing::Limits;
use crate::pricing::Plan;
use crate::pricing::Usage;
use crate::tables::BillingAccount;
use crate::tables::Domain;
use crate::tables::Environment;
use crate::tables::NewBillingAccount;
use crate::tables::Organization;
use crate::tables::Project;

/// The catalogue and `organization`'s plan in it, or why it is unpriced.
fn priced<'s>(
	shipping: &'s Shipping,
	organization: &Organization,
) -> Result<(&'s Catalogue, &'s Plan), String> {
	let Some(catalogue) = &shipping.catalogue else {
		return Err(
			"no price catalogue: this control plane wasn't given one, so nothing is priced"
				.to_owned(),
		);
	};
	let Some(plan) = catalogue.plan(&organization.plan) else {
		return Err(format!(
			"the `{}` plan isn't in the price catalogue",
			organization.plan
		));
	};
	Ok((catalogue, plan))
}

/// The limits of `organization`'s plan, when it is priced.
fn limits(shipping: &Shipping, organization: &Organization) -> Option<Limits> {
	priced(shipping, organization)
		.ok()
		.map(|(_, plan)| plan.limits)
}

fn quantities(usage: &Usage) -> Vec<MeterQuantity> {
	usage
		.iter()
		.map(|(meter, quantity)| {
			MeterQuantity {
				meter: meter.to_string(),
				quantity,
			}
		})
		.collect()
}

fn plan_summary(plan: &Plan) -> PlanSummary {
	PlanSummary {
		id: plan.id.as_str().to_owned(),
		name: plan.name.clone(),
		monthly: plan.monthly,
		included_usage: plan.included_usage,
		allowances: plan
			.allowances
			.iter()
			.map(|(meter, quantity)| {
				MeterQuantity {
					meter: meter.to_string(),
					quantity: *quantity,
				}
			})
			.collect(),
		limits: PlanLimits {
			organizations: plan.limits.organizations,
			projects: plan.limits.projects,
			custom_domains: plan.limits.custom_domains,
			max_size: plan.limits.max_size,
			max_replicas: plan.limits.max_replicas,
			require_sleep: plan.limits.require_sleep,
		},
		spend_cap: effective_cap(plan, None).0,
	}
}

#[expect(
	clippy::unused_async,
	reason = "an operation's body is awaited, whatever it does"
)]
pub(crate) async fn plans(cx: &rpc::QueryCx) -> rpc::Result<PlanList> {
	caller(cx.context())?;
	let services = services(cx.context())?;
	let Some(catalogue) = &services.shipping().catalogue else {
		return Ok(PlanList {
			currency: None,
			plans: Vec::new(),
			prices: Vec::new(),
		});
	};
	Ok(PlanList {
		currency: Some(catalogue.currency().as_str().to_owned()),
		plans: catalogue.plans().iter().map(plan_summary).collect(),
		prices: catalogue
			.prices()
			.map(|(meter, price)| {
				MeterPrice {
					meter: meter.to_string(),
					minor: price.minor(),
					per: price.per(),
				}
			})
			.collect(),
	})
}

/// The cap in force on `plan` for an organisation whose own is `own`, and
/// whether it is the organisation's own. A plan that costs nothing is
/// capped at zero whatever anyone set: nothing pays for more.
fn effective_cap(plan: &Plan, own: Option<i64>) -> (Option<u64>, bool) {
	if plan.is_free() {
		return (Some(0), false);
	}

	match own.and_then(|cap| u64::try_from(cap).ok()) {
		Some(cap) => (Some(cap), true),
		None => (plan.spend_cap, false),
	}
}

async fn own_cap(
	db: &impl leptoskit_db::Read,
	organization: Id<Organization>,
) -> leptoskit_db::Result<Option<i64>> {
	Ok(BillingAccount::query()
		.filter(|row| row.organization_id.eq(organization))
		.first(db)
		.await?
		.and_then(|row| row.spend_cap))
}

/// `usage` on `plan`, priced, with the cap in force.
fn bill(catalogue: &Catalogue, plan: &Plan, usage: Usage, own: Option<i64>) -> UsageBill {
	let estimate = crate::pricing::estimate(
		catalogue,
		plan,
		&EstimateInput {
			usage,
			assumptions: Vec::new(),
		},
	);
	let over = estimate.usage - estimate.included;
	let (spend_cap, own_spend_cap) = effective_cap(plan, own);
	// Usage the catalogue has no price for costs nothing here, and a plan
	// that costs nothing includes none of it: an allowance always has a
	// price.
	let unpriced: Vec<String> = estimate
		.lines
		.iter()
		.filter(|line| line.unit_price.is_none() && line.billable > 0)
		.map(|line| line.meter.to_string())
		.collect();
	let free_and_unpriced = plan.is_free() && !unpriced.is_empty();
	UsageBill {
		currency: estimate.currency.as_str().to_owned(),
		plan_price: estimate.plan_price,
		lines: estimate
			.lines
			.iter()
			.map(|line| {
				CostLine {
					meter: line.meter.to_string(),
					quantity: line.quantity,
					allowance: line.allowance,
					billable: line.billable,
					amount: line.amount,
				}
			})
			.collect(),
		usage: estimate.usage,
		included: estimate.included,
		total: estimate.total,
		over,
		spend_cap,
		own_spend_cap,
		// Past what the plan includes, and at the cap: a cap of zero is
		// reached by the first minor unit, and not before.
		cap_reached: free_and_unpriced || spend_cap.is_some_and(|cap| over > 0 && over >= cap),
		unpriced,
	}
}

pub(crate) async fn usage(
	cx: &rpc::QueryCx,
	organization: &str,
	period: Option<&str>,
) -> rpc::Result<UsageReport> {
	let access =
		authorize::organization(cx.db(), cx.context(), organization, Permission::ViewBilling)
			.await?;
	let month = match period {
		Some(period) if metering::is_period(period) => period.to_owned(),
		Some(period) => {
			return Err(rpc::Error::invalid(format!(
				"invalid_period: `{period}` isn't a month; write one as `2026-11`"
			)));
		}
		None => metering::period(leptoskit_db::now()),
	};
	let services = services(cx.context())?;
	let organization_id = access.organization.id;
	let rows = metering::rows(cx.db(), organization_id, &month).await?;
	let total = metering::total(&rows);
	let slugs = Project::query()
		.filter(|row| row.organization_id.eq(organization_id))
		.all(cx.db())
		.await?;
	let mut projects: Vec<ProjectUsage> = Vec::new();

	for row in &rows {
		let project = row.project_id.uuid();

		if projects.last().is_none_or(|last| last.project != project) {
			projects.push(ProjectUsage {
				project,
				slug: slugs
					.iter()
					.find(|candidate| candidate.id == row.project_id)
					.map(|candidate| candidate.slug.clone()),
				quantities: Vec::new(),
			});
		}

		if let Some(last) = projects.last_mut() {
			last.quantities.push(MeterQuantity {
				meter: row.meter.clone(),
				quantity: u64::try_from(row.quantity).unwrap_or_default(),
			});
		}
	}

	let cost = match priced(services.shipping(), &access.organization) {
		Ok((catalogue, plan)) => {
			UsageCost::Bill(bill(
				catalogue,
				plan,
				total.clone(),
				own_cap(cx.db(), organization_id).await?,
			))
		}
		Err(reason) => UsageCost::Unpriced { reason },
	};
	Ok(UsageReport {
		period: month,
		plan: access.organization.plan.clone(),
		quantities: quantities(&total),
		cost,
		projects,
	})
}

pub(crate) async fn estimate(
	cx: &rpc::QueryCx,
	organization: &str,
	config: &crate::deployments::DeployConfig,
	plan: Option<&str>,
) -> rpc::Result<crate::deployments::CostEstimate> {
	let access = authorize::organization(
		cx.db(),
		cx.context(),
		organization,
		Permission::ViewProjects,
	)
	.await?;
	let services = services(cx.context())?;
	let Some(catalogue) = &services.shipping().catalogue else {
		return Err(rpc::Error::conflict(
			"unpriced: this cloud wasn't given a price catalogue, so nothing is priced",
		));
	};
	let wanted = plan.unwrap_or(access.organization.plan.as_str());
	let plan = catalogue.plan(wanted).ok_or_else(|| {
		rpc::Error::not_found(format!(
			"plan_not_found: this cloud has no `{}` plan",
			super::shown(wanted)
		))
	})?;
	let cloud = crate::release::resolve_all(config)
		.map_err(|error| rpc::Error::invalid(error.to_string()))?;

	Ok(super::deployments::estimate(catalogue, plan, &cloud))
}

pub(crate) async fn set_spend_cap(
	cx: &rpc::MutationCx,
	organization: &str,
	cap: Option<u64>,
) -> rpc::Result<SpendCap> {
	let access = authorize::organization(
		cx.tx(),
		cx.context(),
		organization,
		Permission::ManageBilling,
	)
	.await?;
	let services = services(cx.context())?;
	let (_, plan) = priced(services.shipping(), &access.organization)
		.map_err(|reason| rpc::Error::conflict(format!("unpriced: {reason}")))?;

	if plan.is_free() && cap.is_some_and(|cap| cap > 0) {
		return Err(rpc::Error::forbidden(format!(
			"payment_required: the `{}` plan costs nothing, so nothing pays for usage beyond what \
			 it includes; its spend cap stays at zero",
			plan.id
		)));
	}

	// Usage beyond what a plan includes isn't invoiced yet, so nobody
	// raises their own cap past the one the plan came with.
	if let (Some(cap), Some(most)) = (cap, plan.spend_cap)
		&& cap > most
	{
		return Err(rpc::Error::forbidden(format!(
			"cap_above_plan: the `{}` plan's own spend cap is the most its cap can be set to",
			plan.id
		)));
	}

	let stored = match cap {
		Some(cap) => {
			Some(i64::try_from(cap).map_err(|_| {
				rpc::Error::invalid("invalid_cap: that is more than a spend cap can be")
			})?)
		}
		None => None,
	};
	let organization_id = access.organization.id;

	if let Some(mut row) = BillingAccount::query()
		.filter(|row| row.organization_id.eq(organization_id))
		.first(cx.tx())
		.await?
	{
		row.spend_cap = stored;
		row.save(cx.tx()).await?;
	} else {
		NewBillingAccount {
			organization_id,
			spend_cap: stored,
			customer: None,
			subscription: None,
			status: None,
			period_end: None,
			cancel_at_period_end: None,
		}
		.insert(cx.tx())
		.await?;
	}

	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"billing.set_spend_cap",
		Some(organization_id),
		None,
		Some(match cap {
			Some(cap) => format!("{cap} minor units"),
			None => "the plan's own".to_owned(),
		}),
		&client(cx.context()),
	)
	.await?;
	let (cap, own) = effective_cap(plan, stored);
	Ok(SpendCap { cap, own })
}

/// `Forbidden` (`spend_cap_reached`) once what `organization` used this
/// month costs its spend cap or more beyond what its plan includes. An
/// unpriced organisation is never refused.
///
/// # Errors
///
/// That, or when the database fails.
pub(crate) async fn refuse_over_cap(
	db: &impl leptoskit_db::Read,
	shipping: &Shipping,
	organization: &Organization,
) -> rpc::Result<()> {
	let Ok((catalogue, plan)) = priced(shipping, organization) else {
		return Ok(());
	};
	let month = metering::period(leptoskit_db::now());
	let rows = metering::rows(db, organization.id, &month).await?;
	let bill = bill(
		catalogue,
		plan,
		metering::total(&rows),
		own_cap(db, organization.id).await?,
	);

	if !bill.cap_reached {
		return Ok(());
	}

	// The cap itself is for those who may see the bill: `usage` shows it.
	Err(rpc::Error::forbidden(if plan.is_free() {
		format!(
			"spend_cap_reached: `{}` has used everything the `{}` plan includes this month \
			 ({month}); nothing new is deployed until next month, or on a paid plan. What runs \
			 keeps running",
			organization.slug, plan.id
		)
	} else {
		format!(
			"spend_cap_reached: `{}`'s usage this month ({month}) has reached its spend cap \
			 beyond what the `{}` plan includes; an owner or a billing member can see it with \
			 `usage` and raise it with `set_spend_cap`. What runs keeps running",
			organization.slug, plan.id
		)
	}))
}

/// `Forbidden` (`quota_exceeded`) when `account` has made as many
/// organisations on the starting plan as that plan lets one person make.
/// Every allowance and limit is an organisation's, so without this one
/// person has as many of each as they care to make organisations.
///
/// # Errors
///
/// That, or when the database fails.
pub(crate) async fn refuse_another_organization(
	db: &impl leptoskit_db::Read,
	shipping: &Shipping,
	account: Id<Account>,
) -> rpc::Result<()> {
	let plan = super::personal::STARTING_PLAN;
	let Some(limit) = shipping
		.catalogue
		.as_ref()
		.and_then(|catalogue| catalogue.plan(plan))
		.and_then(|plan| plan.limits.organizations)
	else {
		return Ok(());
	};
	let count = Organization::query()
		.filter(|row| row.created_by.eq(account) & row.plan.eq(plan))
		.count(db)
		.await?;

	if count >= limit {
		return Err(rpc::Error::forbidden(format!(
			"quota_exceeded: one person can make {limit} organisation{} on the `{plan}` plan, \
			 their own included, and you have made that many",
			if limit == 1 { "" } else { "s" },
		)));
	}

	Ok(())
}

/// `Forbidden` (`quota_exceeded`) when `organization` has as many projects
/// as its plan allows.
///
/// # Errors
///
/// That, or when the database fails.
pub(crate) async fn refuse_another_project(
	db: &impl leptoskit_db::Read,
	shipping: &Shipping,
	organization: &Organization,
) -> rpc::Result<()> {
	let Some(Limits {
		projects: Some(limit),
		..
	}) = limits(shipping, organization)
	else {
		return Ok(());
	};
	let organization_id = organization.id;
	let count = Project::query()
		.filter(|row| row.organization_id.eq(organization_id))
		.count(db)
		.await?;

	if count >= limit {
		return Err(rpc::Error::forbidden(format!(
			"quota_exceeded: the `{}` plan allows {limit} project{}, and `{}` has {count}",
			organization.plan,
			if limit == 1 { "" } else { "s" },
			organization.slug
		)));
	}

	Ok(())
}

/// `Forbidden` (`quota_exceeded`) when any process scale exceeds the
/// organisation's plan limits (`max_size`, `max_replicas`, or `require_sleep`).
///
/// # Errors
///
/// `Forbidden` (`quota_exceeded`) when a process asks for more than the plan
/// allows.
pub(crate) fn refuse_scale_above_plan(
	shipping: &Shipping,
	organization: &Organization,
	scale: &crate::projects::EnvironmentScale,
) -> rpc::Result<()> {
	let Some(plan_limits) = limits(shipping, organization) else {
		return Ok(());
	};
	let check_process = |name: &str, process: &crate::projects::ProcessScale| -> rpc::Result<()> {
		if let Some(max_size) = plan_limits.max_size
			&& process.size > max_size
		{
			return Err(rpc::Error::forbidden(format!(
				"quota_exceeded: the `{}` plan allows process sizes up to `{max_size}`, and \
				 `{name}` asks for `{}`",
				organization.plan, process.size
			)));
		}

		if let Some(max_replicas) = plan_limits.max_replicas
			&& (process.max > max_replicas || process.min > max_replicas)
		{
			return Err(rpc::Error::forbidden(format!(
				"quota_exceeded: the `{}` plan allows at most {max_replicas} replica{}, and \
				 `{name}` asks for up to {}",
				organization.plan,
				if max_replicas == 1 { "" } else { "s" },
				process.max.max(process.min)
			)));
		}

		if plan_limits.require_sleep
			&& (process.sleep != crate::size::Sleep::WhenIdle || process.min > 0)
		{
			return Err(rpc::Error::forbidden(format!(
				"quota_exceeded: the `{}` plan requires processes to sleep when idle (`min = 0`, \
				 `sleep = \"when-idle\"`), and `{name}` is set to stay awake",
				organization.plan
			)));
		}

		Ok(())
	};

	check_process("app", &scale.app)?;

	for (name, service) in &scale.services {
		check_process(name, service)?;
	}

	Ok(())
}

/// `Forbidden` (`quota_exceeded`) when `organization` has as many custom
/// domains as its plan allows, over every environment it has.
///
/// # Errors
///
/// That, or when the database fails.
pub(crate) async fn refuse_another_domain(
	db: &impl leptoskit_db::Read,
	shipping: &Shipping,
	organization: &Organization,
) -> rpc::Result<()> {
	let Some(Limits {
		custom_domains: Some(limit),
		..
	}) = limits(shipping, organization)
	else {
		return Ok(());
	};
	let organization_id = organization.id;
	let projects: Vec<Id<Project>> = Project::query()
		.filter(|row| row.organization_id.eq(organization_id))
		.all(db)
		.await?
		.into_iter()
		.map(|row| row.id)
		.collect();
	let environments: Vec<Id<Environment>> = if projects.is_empty() {
		Vec::new()
	} else {
		Environment::query()
			.filter(|row| row.project_id.in_list(projects.clone()))
			.all(db)
			.await?
			.into_iter()
			.map(|row| row.id)
			.collect()
	};
	let count = if environments.is_empty() {
		0
	} else {
		Domain::query()
			.filter(|row| row.environment_id.in_list(environments.clone()))
			.count(db)
			.await?
	};

	if count >= limit {
		return Err(rpc::Error::forbidden(if limit == 0 {
			format!(
				"quota_exceeded: custom domains aren't part of the `{}` plan",
				organization.plan
			)
		} else {
			format!(
				"quota_exceeded: the `{}` plan allows {limit} custom domain{}, and `{}` has that \
				 many over all its projects",
				organization.plan,
				if limit == 1 { "" } else { "s" },
				organization.slug
			)
		}));
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use super::*;
	use crate::pricing::Currency;
	use crate::pricing::Meter;
	use crate::pricing::UnitPrice;
	use crate::slug::Slug;

	fn plan(id: &str, monthly: Option<u64>, included: u64, cap: Option<u64>) -> Plan {
		Plan {
			id: Slug::parse(id).expect("a slug"),
			name: id.to_owned(),
			monthly,
			included_usage: included,
			allowances: BTreeMap::from([(Meter::LinuxBuildMinutes, 10)]),
			limits: Limits::default(),
			spend_cap: cap,
		}
	}

	fn seconds(size: crate::size::Size, count: u64) -> Usage {
		let mut usage = Usage::default();
		usage.add(Meter::Compute(size), count);
		usage
	}

	fn catalogue(plans: Vec<Plan>) -> Catalogue {
		Catalogue::new(
			Currency::parse("usd").expect("a currency"),
			plans,
			// A made-up price: one minor unit a build minute.
			BTreeMap::from([(
				Meter::LinuxBuildMinutes,
				UnitPrice::new(1, 1).expect("a price"),
			)]),
		)
		.expect("a catalogue")
	}

	fn minutes(count: u64) -> Usage {
		let mut usage = Usage::default();
		usage.add(Meter::LinuxBuildMinutes, count);
		usage
	}

	#[test]
	fn a_free_plan_is_capped_at_what_it_includes() {
		let free = plan("hobby", Some(0), 0, Some(500));
		let catalogue = catalogue(vec![free.clone()]);
		assert_eq!(effective_cap(&free, Some(900)), (Some(0), false));
		assert!(!bill(&catalogue, &free, minutes(10), None).cap_reached);
		let over = bill(&catalogue, &free, minutes(11), Some(900));
		assert_eq!(
			(over.over, over.spend_cap, over.cap_reached),
			(1, Some(0), true)
		);
	}

	#[test]
	fn usage_without_a_price_stops_a_free_plan_and_is_named() {
		let free = plan("hobby", Some(0), 0, None);
		let pro = plan("pro", Some(2000), 0, Some(50));
		let catalogue = catalogue(vec![free.clone(), pro.clone()]);
		let used = seconds(crate::size::Size::S, 1);
		let on_free = bill(&catalogue, &free, used.clone(), None);
		assert_eq!(on_free.over, 0);
		assert_eq!(on_free.unpriced, ["compute_seconds_s"]);
		assert!(on_free.cap_reached);
		let on_pro = bill(&catalogue, &pro, used, None);
		assert_eq!(on_pro.unpriced, ["compute_seconds_s"]);
		assert!(!on_pro.cap_reached);
	}

	#[test]
	fn a_paid_plan_is_capped_beyond_what_it_includes() {
		let pro = plan("pro", Some(2000), 100, Some(50));
		let catalogue = catalogue(vec![pro.clone()]);
		// 10 free minutes, then 100 minor units included, then the cap.
		let within = bill(&catalogue, &pro, minutes(159), None);
		assert_eq!((within.usage, within.included, within.over), (149, 100, 49));
		assert!(!within.cap_reached);
		assert_eq!(within.total, 2049);
		let at = bill(&catalogue, &pro, minutes(160), None);
		assert!(at.cap_reached);
		// Its own cap replaces the plan's.
		let raised = bill(&catalogue, &pro, minutes(160), Some(500));
		assert_eq!((raised.spend_cap, raised.own_spend_cap), (Some(500), true));
		assert!(!raised.cap_reached);
		// A plan with no cap of its own has none until one is set.
		let scale = plan("scale", None, 0, None);
		assert!(!bill(&catalogue, &scale, minutes(1_000_000), None).cap_reached);
		assert!(bill(&catalogue, &scale, minutes(1_000_000), Some(5)).cap_reached);
	}
}
