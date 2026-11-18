//! What the console shows that the control plane can't answer yet.
//!
//! `docs/agents/cloud-design.md` §20 lists the operations the design needs.
//! Two of UI 1's don't exist: `deployments::in_flight` (item 3), behind the
//! shipping chip, and `billing::plans` with `usage` (item 9), behind the cost
//! chip and the plan's terms. Each is a trait here with one implementation,
//! [`NotAvailableYet`], which answers that the console can't know, so the
//! screens show their honest "not available yet" states and never a number
//! the cloud didn't give. When the operations land, an implementation that
//! calls them replaces it, and the screens don't change.

/// A release on its way out, as the shipping chip shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShippingRelease {
	/// The project's slug.
	pub project: String,
	/// The release's number.
	pub number: u64,
	/// Where it is, in words: "Building 1:12".
	pub stage: String,
	/// Its sheet.
	pub href: String,
}

/// What's shipping in an organisation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InFlight {
	/// The console can't know yet.
	NotAvailable,
	/// These releases, newest first. Empty when nothing is shipping.
	Releases(Vec<ShippingRelease>),
}

/// What an organisation has spent, in the currency's minor unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Spend {
	/// The console can't know yet.
	NotAvailable,
	/// Spend so far this month, the month's forecast, and the cap.
	Known {
		/// Spent so far this month.
		spent_cents: u64,
		/// The cap.
		cap_cents: u64,
		/// Where the month is heading.
		forecast_cents: u64,
	},
}

/// What a plan includes, in a sentence, as `billing::plans` will say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanTerms {
	/// The console can't know yet.
	NotAvailable,
	/// The plan's name and what it includes.
	Known {
		/// Its name: "Hobby".
		name: String,
		/// What it includes.
		includes: String,
	},
}

/// `deployments::in_flight`: releases in flight in an organisation.
pub trait Deployments: Send + Sync {
	/// What's shipping in `organization`.
	fn in_flight(&self, organization: &str) -> InFlight;
}

/// `billing::plans` and `usage`.
pub trait Billing: Send + Sync {
	/// What `organization` has spent.
	fn spend(&self, organization: &str) -> Spend;
	/// What the plan `slug` includes.
	fn plan(&self, slug: &str) -> PlanTerms;
}

/// The answer until the operations exist: the console can't know.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotAvailableYet;

impl Deployments for NotAvailableYet {
	fn in_flight(&self, _organization: &str) -> InFlight {
		InFlight::NotAvailable
	}
}

impl Billing for NotAvailableYet {
	fn spend(&self, _organization: &str) -> Spend {
		Spend::NotAvailable
	}

	fn plan(&self, _slug: &str) -> PlanTerms {
		PlanTerms::NotAvailable
	}
}

/// A plan's slug as a name: `hobby` is "Hobby".
#[must_use]
pub fn plan_name(slug: &str) -> String {
	let mut chars = slug.chars();
	chars.next().map_or_else(String::new, |first| {
		first.to_uppercase().chain(chars).collect()
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn nothing_is_invented_until_the_operations_exist() {
		assert_eq!(NotAvailableYet.in_flight("acme"), InFlight::NotAvailable);
		assert_eq!(NotAvailableYet.spend("acme"), Spend::NotAvailable);
		assert_eq!(NotAvailableYet.plan("hobby"), PlanTerms::NotAvailable);
	}

	#[test]
	fn plan_slugs_read_as_names() {
		assert_eq!(plan_name("hobby"), "Hobby");
		assert_eq!(plan_name("pro"), "Pro");
		assert_eq!(plan_name(""), "");
	}
}
