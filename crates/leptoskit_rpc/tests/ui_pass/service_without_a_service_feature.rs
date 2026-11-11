//! A crate that calls a service's operations and channels but declares no
//! `service` feature, as an app does, compiles without a warning: nothing
//! the attributes expand to names a feature the crate doesn't have.
#![deny(warnings)]

use leptoskit_rpc as rpc;

/// Lives in the `analytics` service.
#[rpc::query(service = "analytics")]
pub async fn summary(cx: &rpc::QueryCx, days: u32) -> rpc::Result<u32> {
	let _ = cx;
	Ok(days)
}

/// Lives in the `analytics` service.
#[rpc::mutation(service = "analytics")]
pub async fn capture(cx: &rpc::MutationCx, event: String) -> rpc::Result<()> {
	let _ = (cx, event);
	Ok(())
}

/// Rolls up in the `analytics` service.
#[rpc::job(service = "analytics")]
pub async fn roll_up(cx: &rpc::JobCx) -> rpc::Result<()> {
	let _ = cx;
	Ok(())
}

/// Hosted by the `analytics` service.
#[rpc::channel(message = String, authorize = may_join, service = "analytics")]
pub struct Dashboard {
	/// Which dashboard.
	pub name: String,
}

async fn may_join(cx: &rpc::QueryCx, dashboard: &Dashboard) -> rpc::Result<()> {
	let _ = (cx, dashboard);
	Ok(())
}

fn main() {
	use rpc::Channel as _;
	use rpc::Operation as _;

	assert_eq!(Summary::SERVICE, Some("analytics"));
	assert_eq!(Capture::SERVICE, Some("analytics"));
	assert_eq!(<RollUp as rpc::Operation>::SERVICE, Some("analytics"));
	assert_eq!(Dashboard::SERVICE, Some("analytics"));
	let _ = (
		Summary { days: 7 },
		Capture {
			event: String::new(),
		},
		RollUp {},
	);
	let _ = Dashboard {
		name: String::new(),
	};
	let _ = may_join;
}
