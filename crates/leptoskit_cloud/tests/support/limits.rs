//! How often a caller may ask, written once and run on SQLite
//! (`deployments.rs`) and Postgres (`postgres.rs`).

use std::time::Duration;

use leptoskit_cloud::server::ControlConfig;
use leptoskit_cloud::server::Rate;
use serde_json::json;

use super::App;
use super::As;
use super::subscriptions::Till;

fn an_hour(max: u32) -> Rate {
	Rate {
		max,
		window: Duration::from_hours(1),
	}
}

/// Rates a scenario reaches in a few calls.
pub fn slow(config: &mut ControlConfig) {
	config.rate_limits.person = an_hour(12);
	config.rate_limits.token = an_hour(3);
	config.rate_limits.organization_tokens = an_hour(4);
	config.rate_limits.anonymous = an_hour(2);
}

/// Asks `whoami` as `caller` until it is refused, and answers how many
/// times it was answered first.
async fn until_refused(app: &App, caller: As<'_>, most: u32) -> u32 {
	for answered in 0..=most {
		let answer = app.call("whoami", json!({}), caller).await;

		if answer.status == 429 {
			assert!(answer.text.contains("rate_limited"), "{}", answer.text);
			return answered;
		}
	}

	panic!("{most} calls and none refused");
}

/// Each caller has a rate of their own: a person over every way they are
/// signed in, a token, and an address nobody is signed in from. One of
/// them using theirs up slows nobody else.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_caller_who_asks_too_often_waits(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let bob = app.sign_in("bob@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let theirs = app.personal(As::Session(&bob)).await;
	let (token, _) = app.token(As::Session(&ana), &org, &["read"], None).await;

	// Ana has asked a few times already, and may ask twelve times in all.
	let answered = until_refused(app, As::Session(&ana), 12).await;
	assert!((1..12).contains(&answered), "{answered}");
	// Refused is refused for everything of hers, until the window ends.
	app.call("plans", json!({}), As::Session(&ana))
		.await
		.refused(429, "rate_limited");
	// A second session of hers is the same person.
	let again = app.sign_in("ana@example.com").await;
	app.call("whoami", json!({}), As::Session(&again))
		.await
		.refused(429, "rate_limited");

	// Bob isn't slowed, and neither is her token: it has its own three.
	app.call("whoami", json!({}), As::Session(&bob))
		.await
		.expect(200);
	let projects = || {
		app.call(
			"projects",
			json!({ "organization": org }),
			As::Bearer(&token),
		)
	};

	for _ in 0..3 {
		projects().await.expect(200);
	}

	projects().await.refused(429, "rate_limited");

	// A second token doesn't bring three more: the organisation's tokens
	// have four between them.
	let (second, _) = app.token(As::Session(&bob), &theirs, &["read"], None).await;
	let (third, _) = app.token(As::Session(&bob), &theirs, &["read"], None).await;
	let listed = |token| {
		app.call(
			"projects",
			json!({ "organization": theirs }),
			As::Bearer(token),
		)
	};

	for _ in 0..3 {
		listed(&second).await.expect(200);
	}

	listed(&third).await.expect(200);
	listed(&third).await.refused(429, "rate_limited");

	// Nobody, and a credential that is nobody's, share the address's two.
	app.call("whoami", json!({}), As::Nobody).await.expect(401);
	app.call("whoami", json!({}), As::Bearer("lkc_000000000000_made_up"))
		.await
		.expect(401);
	app.call("whoami", json!({}), As::Nobody)
		.await
		.refused(429, "rate_limited");
	app.call("whoami", json!({}), As::Bearer("lkc_000000000001_made_up"))
		.await
		.refused(429, "rate_limited");
	// Which slows nobody who is somebody.
	app.call("whoami", json!({}), As::Session(&bob))
		.await
		.expect(200);
}

/// A control plane's configuration that takes payment through `till` and
/// hands an organisation two payment pages an hour.
pub fn two_pages(till: &Till) -> impl FnOnce(&mut ControlConfig) + use<> {
	let taking = super::subscriptions::taking_payment(till);
	move |config| {
		taking(config);
		config.rate_limits.payment_pages = an_hour(2);
	}
}

/// Checkout and portal pages are calls to the payment provider with the
/// control plane's own key, so an organisation gets only so many.
pub async fn payment_pages_are_handed_out_slowly(app: &App, till: &Till) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let checkout = |org: &str, caller| {
		app.call(
			"checkout",
			json!({ "organization": org, "plan": "pro" }),
			caller,
		)
	};
	checkout(&org, As::Session(&ana)).await.expect(200);
	checkout(&org, As::Session(&ana)).await.expect(200);
	checkout(&org, As::Session(&ana))
		.await
		.refused(429, "rate_limited");
	app.call("portal", json!({ "organization": org }), As::Session(&ana))
		.await
		.refused(429, "rate_limited");
	assert_eq!(till.checkouts().len(), 2);
	assert_eq!(till.portals().len(), 0);

	// Another organisation's pages are its own.
	let bob = app.sign_in("bob@example.com").await;
	let theirs = app.personal(As::Session(&bob)).await;
	checkout(&theirs, As::Session(&bob)).await.expect(200);
	// And someone who may not ask uses up none of them.
	let dev = app
		.join(As::Session(&bob), &theirs, "dev@example.com", "developer")
		.await;

	for _ in 0..3 {
		checkout(&theirs, As::Session(&dev))
			.await
			.refused(403, "insufficient_role");
	}

	checkout(&theirs, As::Session(&bob)).await.expect(200);
	checkout(&theirs, As::Session(&bob))
		.await
		.refused(429, "rate_limited");
}
