//! Signing in: one button that goes to the main site, and the page for
//! someone the main site signed in who isn't an admin. The pages crate
//! draws it; this builds what it draws from.

use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::Response;
use leptoskit_admin_pages::Model;
use leptoskit_admin_pages::model::Page;
use leptoskit_admin_pages::model::Viewer;
pub(crate) use leptoskit_admin_pages::model::signin::Outcome;
use leptoskit_admin_pages::model::signin::SignIn;
use leptoskit_admin_pages::pages::host;

use crate::Admin;
use crate::server;

/// The sign-in page saying `outcome`: refused with 403, otherwise 200.
pub(crate) async fn render(admin: &Admin, outcome: Outcome, headers: &HeaderMap) -> Response {
	let inner = admin.inner();
	let status = if matches!(outcome, Outcome::Refused(_)) {
		StatusCode::FORBIDDEN
	} else {
		StatusCode::OK
	};
	let model = Model {
		// Nobody is signed in yet, so the viewer is only the admin's own.
		viewer: Viewer {
			app_name: inner.app_name.clone(),
			environment: inner.environment,
			simulated: inner.simulated_infrastructure,
			who: String::new(),
			workings_mode: false,
			appearance: server::appearance(headers),
			projects: Vec::new(),
			progress: (0, 0),
			pulse: None,
		},
		page: Page::SignIn(Box::new(SignIn {
			site: host(&inner.main_site.issuer).to_owned(),
			button: inner.button.clone(),
			roles: inner.roles.clone(),
			outcome,
		})),
		open: None,
	};
	let mut response = inner.client.render(model).await;
	*response.status_mut() = status;
	response
}
