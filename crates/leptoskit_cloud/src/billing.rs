//! What an organisation used, what it costs, and how much it may spend.
//!
//! The control plane counts usage itself, per project and calendar month
//! (UTC): the minutes its build workers held a build, and the seconds an
//! environment's replicas were ready, from its cluster worker's reports.
//! [`usage`] prices a month's counts with the operator's catalogue and the
//! organisation's plan, the same way an estimate is priced
//! ([`crate::pricing`]).
//!
//! A *spend cap* is the most a month's usage may cost beyond what the plan
//! includes. Each plan has its own in the catalogue, and an organisation
//! may set another with [`set_spend_cap`]. On a plan that costs nothing the
//! cap is always zero: nothing pays for more. Once the month's usage is
//! past the cap, `deploy` answers `spend_cap_reached`. Nothing is stopped
//! or deleted.
//!
//! A plan's limits are counted when something is made: `create_project` and
//! `attach_domain` answer `quota_exceeded` at the plan's limit.
//!
//! Nothing here charges anyone: payment arrives in a later slice.

use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::deployments::CostLine;

/// A quantity of one meter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MeterQuantity {
	/// The meter's code, such as `build_minutes_linux`.
	pub meter: String,
	pub quantity: u64,
}

/// What an organisation on a plan may have at once. `None` is no limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanLimits {
	pub projects: Option<u64>,
	pub custom_domains: Option<u64>,
}

/// A plan the cloud offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanSummary {
	/// Its slug, such as `hobby`.
	pub id: String,
	pub name: String,
	/// Its monthly price in minor units, or `None` when it is priced by
	/// agreement.
	pub monthly: Option<u64>,
	/// Usage, in minor units, the monthly price already pays for.
	pub included_usage: u64,
	/// Quantities each month that cost nothing.
	pub allowances: Vec<MeterQuantity>,
	pub limits: PlanLimits,
	/// The spend cap an organisation on it has until it sets its own.
	pub spend_cap: Option<u64>,
}

/// One meter's price: `minor` minor units for every `per` units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MeterPrice {
	pub meter: String,
	pub minor: u64,
	pub per: u64,
}

/// The plans and prices this cloud offers. Without a price catalogue there
/// are none, and nothing is priced or limited.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanList {
	/// The currency every amount is in, such as `usd`.
	pub currency: Option<String>,
	pub plans: Vec<PlanSummary>,
	pub prices: Vec<MeterPrice>,
}

/// What one project used in a month.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectUsage {
	pub project: Uuid,
	/// The project's slug, while it exists.
	pub slug: Option<String>,
	pub quantities: Vec<MeterQuantity>,
}

/// A month's usage, priced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageBill {
	pub currency: String,
	/// The plan's monthly price, or `None` when it is priced by agreement.
	pub plan_price: Option<u64>,
	pub lines: Vec<CostLine>,
	/// The lines' amounts added up.
	pub usage: u64,
	/// The part of `usage` the plan's price already pays for.
	pub included: u64,
	/// The plan's price, plus usage beyond what it includes.
	pub total: u64,
	/// Usage beyond what the plan includes: what the spend cap limits.
	pub over: u64,
	/// The spend cap in force, or `None` when there is none.
	pub spend_cap: Option<u64>,
	/// Whether the organisation set that cap itself.
	pub own_spend_cap: bool,
	/// Whether `over` has reached the cap, so `deploy` is refused.
	pub cap_reached: bool,
}

/// What a month's usage costs, or why nothing can say.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UsageCost {
	Bill(UsageBill),
	/// The control plane has no price catalogue, or the catalogue doesn't
	/// have the organisation's plan.
	Unpriced {
		reason: String,
	},
}

/// What an organisation used in one calendar month (UTC).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageReport {
	/// The month, as `2026-11`.
	pub period: String,
	/// The plan the organisation is on now.
	pub plan: String,
	/// Every project's usage added up, per meter.
	pub quantities: Vec<MeterQuantity>,
	pub cost: UsageCost,
	/// By project, largest first is not promised: by the project's ID.
	pub projects: Vec<ProjectUsage>,
}

/// An organisation's spend cap, after a change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SpendCap {
	/// The cap in force, in minor units, or `None` when there is none.
	pub cap: Option<u64>,
	/// Whether the organisation set it, rather than its plan.
	pub own: bool,
}

/// The plans and prices this cloud offers.
///
/// # Errors
///
/// `Unauthorized` when nobody is signed in.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "billing")]
#[rpc::query]
pub async fn plans(cx: &rpc::QueryCx) -> rpc::Result<PlanList> {
	crate::server::billing::plans(cx).await
}

/// What `organization` used in `period`, a month as `2026-11`, or in this
/// month without one, and what it costs on the plan it is on now.
///
/// # Errors
///
/// `Invalid` (`invalid_period`) for a period that isn't a month.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "billing")]
#[rpc::query]
pub async fn usage(
	cx: &rpc::QueryCx,
	organization: String,
	period: Option<String>,
) -> rpc::Result<UsageReport> {
	crate::server::billing::usage(cx, &organization, period.as_deref()).await
}

/// Sets the most a month's usage may cost `organization` beyond what its
/// plan includes, in minor units; `None` goes back to the plan's own cap.
/// Owners and billing members only, and never a token.
///
/// # Errors
///
/// `Conflict` (`unpriced`) when the control plane has no price for the
/// organisation's plan, so a cap would limit nothing; `Forbidden`
/// (`payment_required`) for a cap above zero on a plan that costs nothing.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "billing")]
#[rpc::mutation]
pub async fn set_spend_cap(
	cx: &rpc::MutationCx,
	organization: String,
	cap: Option<u64>,
) -> rpc::Result<SpendCap> {
	crate::server::billing::set_spend_cap(cx, &organization, cap).await
}
