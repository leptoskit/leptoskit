//! The cloud's blocking HTTP boundary. Secrets never enter diagnostic chains.

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use serde::de::DeserializeOwned;

use crate::publish::secrets::Secret;

pub(super) fn origin(value: &str) -> Result<String> {
	let url = url::Url::parse(value).map_err(|_| anyhow::anyhow!("Invalid cloud URL."))?;
	ensure!(
		(url.scheme() == "https" || (url.scheme() == "http" && loopback(&url)))
			&& url.path() == "/"
			&& url.query().is_none()
			&& url.fragment().is_none()
			&& url.username().is_empty()
			&& url.password().is_none(),
		"Cloud URL needs an HTTPS origin, or HTTP on loopback, without a path or credentials."
	);
	Ok(url.origin().ascii_serialization())
}

fn loopback(url: &url::Url) -> bool {
	match url.host() {
		Some(url::Host::Domain("localhost")) => true,
		Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
		Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
		_ => false,
	}
}

pub(super) fn agent(origin: &str) -> ureq::Agent {
	let mut config = ureq::Agent::config_builder()
		.timeout_global(Some(std::time::Duration::from_secs(30)))
		.max_redirects(0)
		.http_status_as_error(false);

	if url::Url::parse(origin).is_ok_and(|url| loopback(&url)) {
		config = config.proxy(None);
	}

	config.build().into()
}

/// A rejected cloud operation, with the HTTP status and stable code.
#[derive(Debug)]
pub struct Rejected {
	/// The HTTP status returned by the control plane.
	pub status: u16,
	/// The stable machine-readable error code.
	pub code: String,
	message: String,
}

impl std::fmt::Display for Rejected {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(message(&self.code).unwrap_or(&self.message))
	}
}

impl std::error::Error for Rejected {}

fn message(code: &str) -> Option<&'static str> {
	Some(match code {
		"sign_in_required" => "You're not signed in. Run `lk cloud login`.",
		"person_required" => {
			"This needs a person signed in with `lk cloud login`; unset LEPTOSKIT_CLOUD_TOKEN."
		}
		"organization_not_found" => "Organization not found. Check `lk cloud org list`.",
		"project_not_found" => "Project not found. Check `lk cloud project list`.",
		"environment_not_found" => "Environment not found. Check `lk cloud env list`.",
		"insufficient_role" => {
			"Your role doesn't allow this. Ask an organization owner for access."
		}
		"insufficient_scope" => {
			"This token doesn't allow this. Use a token with the required scope."
		}
		"stale_revision" => {
			"Someone else changed this environment. Retry to read its current revision."
		}
		"confirmation_mismatch" => "Confirmation didn't match. Pass the exact slug with --confirm.",
		"production" => "Production cannot be deleted. Delete the project to remove it.",
		"slug_taken" => "That slug is in use. Choose another slug.",
		"name_taken" => "That name is in use. Choose another name.",
		"already_member" => {
			"That person is already a member. Use `lk cloud org set-role` to change their role."
		}
		"last_owner" => "This is the last owner. Make another member an owner first.",
		"token_too_strong" => {
			"Your role cannot grant those scopes. Choose narrower scopes or ask an owner."
		}
		"project_token" => {
			"This token is bound to one project. Use an organization token for this operation."
		}
		"invalid_email" => "That email address isn't valid. Check the address and retry.",
		"invalid_name" => "That name isn't valid. Choose a nonempty name and retry.",
		"invalid_slug" => {
			"That slug isn't valid. Use 3 to 40 lowercase letters, digits and hyphens, starting \
			 with a letter."
		}
		"invalid_detected" => {
			"Detected targets or variable names aren't valid. Send names only and retry."
		}
		"invalid_environment" => {
			"That environment isn't valid. Use a staging or preview name and retry."
		}
		"invalid_region" => {
			"That region isn't valid. Check the cloud's available regions and retry."
		}
		"invalid_scale" => {
			"That scale isn't valid. Check minimum, maximum and sleep settings and retry."
		}
		"invalid_lifetime" => "That token lifetime isn't valid. Use --expires 30, 90 or 365.",
		"invalid_scopes" => {
			"A token needs a scope. Pass --scope read, deploy, manage, publish, billing or admin."
		}
		"invitation_invalid" => {
			"That invitation is no longer valid. Ask an organization owner for a new invitation."
		}
		"invitation_not_found" => "Invitation not found. Check `lk cloud org invitations`.",
		"member_not_found" => "Member not found. Check `lk cloud org members`.",
		"token_not_found" => "Token not found. Check `lk cloud token list`.",
		"wrong_email" => "Sign in with the invitation's verified email address and retry.",
		"too_many_invitations" => {
			"This organization has too many pending invitations. Revoke an invitation and retry."
		}
		"too_many_projects" => {
			"This organization has reached its project limit. Delete an unused project and retry."
		}
		"too_many_environments" => {
			"This project has reached its environment limit. Delete an unused environment and \
			 retry."
		}
		_ => return None,
	})
}

pub(super) struct Reply {
	pub status: u16,
	pub body: String,
}

impl Reply {
	pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
		serde_json::from_str(&self.body).map_err(|_| anyhow::anyhow!("Invalid cloud response."))
	}

	pub fn result<T: DeserializeOwned>(&self, secrets: &[&Secret]) -> Result<T> {
		if (200..300).contains(&self.status) {
			return self.json();
		}

		let mut body = self.body.clone();
		for secret in secrets {
			if !secret.expose().is_empty() {
				body = body.replace(secret.expose(), "[redacted]");
			}
		}
		let wire = body.split_once('|').map_or(body.as_str(), |(_, text)| text);
		let (code, text) = wire.split_once(':').unwrap_or(("request_rejected", wire));
		let mut message = text.trim().to_owned();

		for secret in secrets {
			if !secret.expose().is_empty() {
				message = message.replace(secret.expose(), "[redacted]");
			}
		}

		Err(Rejected {
			status: self.status,
			code: code.to_owned(),
			message,
		}
		.into())
	}
}

pub(super) fn read(
	response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<Reply> {
	let mut response = response.map_err(|_| {
		anyhow::anyhow!("Cloud request failed. Check your connection and cloud URL.")
	})?;
	let status = response.status().as_u16();
	let body = response
		.body_mut()
		.with_config()
		.limit(2 * 1024 * 1024)
		.read_to_string()
		.context("Could not read cloud response.")?;
	Ok(Reply { status, body })
}
