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
		match message(&self.code) {
			Some(message) => f.write_str(message),
			None => f.write_str(&super::printable(&self.message).replace('\n', " ")),
		}
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
		"variable_not_found" => "Variable not found. Check `lk cloud var list`.",
		"secret_not_found" => "Secret not found. Check `lk cloud secret list`.",
		"name_is_secret" => {
			"A secret has that name. Unset it with `lk cloud secret unset` first, or choose \
			 another name."
		}
		"name_is_variable" => {
			"A variable has that name. Unset it with `lk cloud var unset` first, or choose another \
			 name."
		}
		"too_many_variables" => "This environment has too many variables. Unset one and retry.",
		"too_many_secrets" => "This environment has too many secrets. Unset one and retry.",
		"value_empty" => "The value is empty. Pipe in the value to store.",
		"value_too_long" => "The value is too long. Store a shorter value.",
		"value_not_text" => "The value isn't text. Encode it as base64 first.",
		"invalid_domain" => {
			"That hostname isn't valid here. Use a domain you own, such as app.example.com."
		}
		"domain_not_found" => "Domain not found. Check `lk cloud domain list`.",
		"domain_not_verified" => {
			"The challenge record isn't there yet. Publish the TXT record from `lk cloud domain \
			 list`, wait for DNS, and retry."
		}
		"domain_taken" => "Another environment holds that hostname. Detach it there first.",
		"dns_unavailable" => "The DNS lookup failed. Retry in a minute.",
		"too_many_domains" => "This environment has too many domains. Remove one and retry.",
		"deployment_in_flight" => {
			"A deployment of this environment is under way. Wait for it to finish, then retry."
		}
		"rollback_unavailable" => {
			"That deployment can't be returned to: it never went live, its image is gone, or the \
			 database has migrations it doesn't know."
		}
		"deployment_not_found" => "Deployment not found. Check `lk cloud deployments`.",
		"no_database" => "This environment has no database.",
		"database_not_ready" => "The database isn't ready until a release that uses it is live.",
		"backup_in_progress" => "A backup or restore is under way. Wait for it, then retry.",
		"too_many_backups" => {
			"This environment keeps the most backups it can. Wait for one to expire."
		}
		"backup_not_found" => "Backup not found. Check `lk cloud db backups`.",
		"backup_unavailable" => "That backup isn't complete, or has expired. Choose another.",
		"restore_unavailable" => {
			"That backup holds migrations the live deployment doesn't know, or the database's \
			 migrations are uncertain."
		}
		"invalid_stores" => {
			"Name each store once: chrome, edge, firefox, app_store, play or solana, each \
			 configured in leptoskit.toml."
		}
		"archive_not_found" => "The cloud doesn't have that archive. Retry to upload it again.",
		"archive_too_large" => {
			"The project is too large to upload. Add what the build doesn't need to \
			 .leptoskitignore."
		}
		"archive_empty" => "The archive is empty. Run this in the project's directory.",
		"storage_full" => {
			"The organization's archive storage is full. Retry once older archives expire."
		}
		"upload_not_found" => "The upload wasn't found. Retry to upload again.",
		"upload_mismatch" => "The uploaded bytes didn't match. Retry to upload again.",
		"publish_in_flight" => {
			"A publish of this environment is queued or running. Follow it with `lk cloud \
			 publications list`."
		}
		"publication_not_found" => "Publish not found. Check `lk cloud publications list`.",
		"invalid_period" => "Use a month such as 2026-11.",
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
			body = redact(&body, secret);
		}
		let wire = body.split_once('|').map_or(body.as_str(), |(_, text)| text);
		let (code, text) = wire.split_once(':').unwrap_or(("request_rejected", wire));
		let mut message = text.trim().to_owned();

		for secret in secrets {
			message = redact(&message, secret);
		}

		Err(Rejected {
			status: self.status,
			code: code.to_owned(),
			message,
		}
		.into())
	}
}

/// `text` without `secret`, as is and as JSON would escape it.
fn redact(text: &str, secret: &Secret) -> String {
	let value = secret.expose();
	if value.is_empty() {
		return text.to_owned();
	}
	let mut text = text.replace(value, "[redacted]");
	if let Ok(quoted) = serde_json::to_string(value) {
		let escaped = &quoted[1..quoted.len() - 1];
		if escaped != value {
			text = text.replace(escaped, "[redacted]");
		}
	}
	text
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

/// Puts an archive's bytes where an upload ticket says: `PUT` with exactly
/// its headers, to an `https://` address or `http://` on loopback, never
/// following a redirect. The address is the credential, so no error
/// repeats it.
pub(super) fn put(
	ticket: &leptoskit_cloud::deployments::UploadTicket,
	bytes: &[u8],
	cloud_is_loopback: bool,
) -> Result<()> {
	let url =
		url::Url::parse(&ticket.url).map_err(|_| anyhow::anyhow!("Invalid upload address."))?;
	ensure!(
		ticket.method.eq_ignore_ascii_case("PUT")
			&& (url.scheme() == "https" || (url.scheme() == "http" && loopback(&url)))
			&& url.username().is_empty()
			&& url.password().is_none()
			&& (cloud_is_loopback || !loopback(&url)),
		"The cloud gave an upload address this CLI won't send to."
	);
	let mut config = ureq::Agent::config_builder()
		.timeout_global(Some(std::time::Duration::from_mins(30)))
		.max_redirects(0)
		.http_status_as_error(false);
	if loopback(&url) {
		config = config.proxy(None);
	}
	let agent: ureq::Agent = config.build().into();
	// Send to the address that was checked, as `url` serializes it, never
	// the raw string another parser might read differently.
	let mut request = agent.put(url.as_str());
	for (name, value) in &ticket.headers {
		request = request.header(name.as_str(), value.as_str());
	}
	let status = request
		.send(bytes)
		.map_err(|_| {
			anyhow::anyhow!("Uploading the archive failed. Check your connection and retry.")
		})?
		.status()
		.as_u16();
	ensure!(
		(200..300).contains(&status),
		"The archive's store refused the upload (HTTP {status}). Retry; the cloud will ask for it \
		 again."
	);
	Ok(())
}
