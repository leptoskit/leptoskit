//! Typed calls to the account control plane and its first-party device login.

use std::path::Path;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::bail;
use anyhow::ensure;
use leptoskit_cloud::account;
use leptoskit_cloud::projects;
use leptoskit_cloud::tokens;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::credentials::Session;
use super::credentials::Store;
pub use super::transport::Rejected;
use super::transport::Reply;
use super::transport::{self};
use crate::publish::secrets::Secret;

/// A connection to one cloud origin. Debug never exposes credentials.
pub struct Client {
	origin: String,
	agent: ureq::Agent,
	store: Store,
	api_token: Option<Secret>,
}

#[derive(Deserialize)]
struct Discovery {
	cli_client_id: String,
	scope: String,
	rpc: String,
}

#[derive(Deserialize)]
struct OAuthDiscovery {
	#[serde(rename = "device_authorization_endpoint")]
	device_authorization: String,
	#[serde(rename = "token_endpoint")]
	token: String,
	#[serde(rename = "revocation_endpoint")]
	revocation: String,
}

/// What login shows while waiting for approval. Never contains the device code.
#[derive(Debug)]
pub struct Verification {
	/// Where to enter the user code.
	pub url: String,
	/// The short code to enter in the browser.
	pub code: String,
}

#[derive(Deserialize)]
struct DeviceAuthorization {
	#[serde(with = "super::credentials::secret")]
	device_code: Secret,
	user_code: String,
	verification_uri: String,
	expires_in: u64,
	#[serde(default = "default_interval")]
	interval: u64,
}

const fn default_interval() -> u64 {
	5
}

#[derive(Deserialize)]
struct Tokens {
	#[serde(with = "super::credentials::secret")]
	access_token: Secret,
	#[serde(with = "super::credentials::secret")]
	refresh_token: Secret,
	expires_in: u64,
}

fn now() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |time| time.as_secs())
}

impl Client {
	/// Connects with an explicit credential directory and optional CI token.
	///
	/// # Errors
	/// When the URL isn't a secure origin.
	pub fn new(origin: &str, directory: &Path, api_token: Option<Secret>) -> Result<Self> {
		let origin = transport::origin(origin)?;
		Ok(Self {
			agent: transport::agent(&origin),
			store: Store::new(directory, &origin),
			origin,
			api_token,
		})
	}

	/// Whether this invocation uses a CI token rather than a signed-in person.
	#[must_use]
	pub fn uses_api_token(&self) -> bool {
		self.api_token.is_some()
	}

	fn discovery(&self) -> Result<Discovery> {
		let reply = transport::read(
			self.agent
				.get(format!("{}/.well-known/leptoskit-cloud", self.origin))
				.call(),
		)?;
		let discovery: Discovery = reply.result(&[]).context("Cloud discovery failed")?;
		ensure!(
			discovery.rpc.starts_with('/')
				&& !discovery.rpc.starts_with("//")
				&& !discovery.rpc.contains(['?', '#']),
			"Invalid cloud RPC path."
		);
		Ok(discovery)
	}

	fn form(&self, path: &str, fields: &[(&str, &str)]) -> Result<Reply> {
		transport::read(
			self.agent
				.post(format!("{}{path}", self.origin))
				.send_form(fields.iter().copied()),
		)
	}

	fn oauth(&self) -> Result<OAuthDiscovery> {
		let mut discovery: OAuthDiscovery = transport::read(
			self.agent
				.get(format!("{}/.well-known/openid-configuration", self.origin))
				.call(),
		)?
		.result(&[])?;
		for endpoint in [
			&mut discovery.device_authorization,
			&mut discovery.token,
			&mut discovery.revocation,
		] {
			let url = url::Url::parse(endpoint).context("Invalid OAuth endpoint.")?;
			ensure!(
				url.origin().ascii_serialization() == self.origin
					&& url.username().is_empty()
					&& url.password().is_none()
					&& url.query().is_none()
					&& url.fragment().is_none(),
				"OAuth endpoint must belong to this cloud."
			);
			url.path().clone_into(endpoint);
		}
		Ok(discovery)
	}

	/// Runs device login, calling `show` before polling and replacing the
	/// stored session.
	///
	/// # Errors
	/// When discovery, approval, or private credential storage fails.
	pub fn login(&self, show: impl FnOnce(&Verification) -> Result<()>) -> Result<()> {
		let discovery = self.discovery()?;
		let oauth = self.oauth()?;
		let device: DeviceAuthorization = self
			.form(
				&oauth.device_authorization,
				&[
					("client_id", &discovery.cli_client_id),
					("scope", &discovery.scope),
				],
			)?
			.result(&[])
			.context("Device authorization failed")?;
		// Only open a same-origin verification URL. Discovery cannot redirect
		// credentials.
		let verification =
			url::Url::parse(&device.verification_uri).context("Invalid verification URL.")?;
		ensure!(
			verification.origin().ascii_serialization() == self.origin,
			"Verification URL must belong to this cloud."
		);
		show(&Verification {
			url: device.verification_uri,
			code: device.user_code,
		})?;
		let deadline = Instant::now()
			.checked_add(Duration::from_secs(device.expires_in))
			.context("Invalid device login expiry.")?;
		let mut interval = device.interval.max(1);

		loop {
			if deadline.saturating_duration_since(Instant::now()) <= Duration::from_secs(interval) {
				bail!("Login expired. Run `lk cloud login` again.");
			}
			std::thread::sleep(Duration::from_secs(interval));
			let reply = self.form(
				&oauth.token,
				&[
					("client_id", &discovery.cli_client_id),
					("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
					("device_code", device.device_code.expose()),
				],
			)?;
			if reply.status == 200 {
				let tokens: Tokens = reply.json()?;
				let _lock = self.store.lock()?;
				self.store.save(&Session {
					client_id: discovery.cli_client_id,
					access: tokens.access_token,
					refresh: tokens.refresh_token,
					expires_at: now().saturating_add(tokens.expires_in),
				})?;
				return Ok(());
			}
			#[derive(Deserialize)]
			struct OAuthError {
				error: String,
			}
			let error: OAuthError = reply.json()?;
			match error.error.as_str() {
				"authorization_pending" => {}
				"slow_down" => interval = interval.saturating_add(5),
				"access_denied" => bail!("Login was denied. Run `lk cloud login` to try again."),
				"expired_token" => bail!("Login expired. Run `lk cloud login` again."),
				_ => bail!("Cloud login failed. Run `lk cloud login` again."),
			}
		}
	}

	/// Revokes the refresh token and removes the local session even if offline.
	/// Returns whether server revocation succeeded.
	///
	/// # Errors
	/// When local credentials cannot be safely read or deleted.
	pub fn logout(&self) -> Result<bool> {
		let _lock = self.store.lock()?;
		let session = self.store.load()?;
		let revoked = session.as_ref().is_none_or(|session| {
			self.oauth()
				.and_then(|oauth| {
					self.form(
						&oauth.revocation,
						&[
							("client_id", &session.client_id),
							("token", session.refresh.expose()),
							("token_type_hint", "refresh_token"),
						],
					)
				})
				.is_ok_and(|reply| (200..300).contains(&reply.status))
		});
		self.store.delete()?;
		Ok(revoked)
	}

	fn access(&self, refused: Option<&Secret>) -> Result<Secret> {
		if let Some(token) = &self.api_token {
			return Ok(token.clone());
		}
		let session = self
			.store
			.load()?
			.context("You're not signed in. Run `lk cloud login`.")?;
		if refused.is_none() && session.expires_at > now().saturating_add(30) {
			return Ok(session.access);
		}
		let _lock = self.store.lock()?;
		// A different process may have rotated while we waited. Never reuse its
		// refresh token.
		let session = self
			.store
			.load()?
			.context("You're not signed in. Run `lk cloud login`.")?;
		if session.expires_at > now().saturating_add(30)
			&& refused.is_none_or(|token| token.expose() != session.access.expose())
		{
			return Ok(session.access);
		}
		let oauth = self.oauth()?;
		let reply = self.form(
			&oauth.token,
			&[
				("client_id", &session.client_id),
				("grant_type", "refresh_token"),
				("refresh_token", session.refresh.expose()),
			],
		)?;
		if reply.status != 200 {
			if (400..500).contains(&reply.status) {
				self.store.delete()?;
			}
			bail!("Could not refresh cloud login. Run `lk cloud login` again.");
		}
		let tokens: Tokens = reply.json()?;
		let session = Session {
			client_id: session.client_id,
			access: tokens.access_token,
			refresh: tokens.refresh_token,
			expires_at: now().saturating_add(tokens.expires_in),
		};
		self.store.save(&session)?;
		Ok(session.access)
	}

	fn call<A: Serialize, T: DeserializeOwned>(&self, name: &str, args: &A) -> Result<T> {
		let discovery = self.discovery()?;
		let token = self.access(None)?;
		let send = |token: &Secret| {
			let mut authorization =
				ureq::http::HeaderValue::from_str(&format!("Bearer {}", token.expose())).map_err(
					|_| {
						anyhow::anyhow!(
							"Invalid cloud credential. Run `lk cloud login` or check \
							 LEPTOSKIT_CLOUD_TOKEN."
						)
					},
				)?;
			authorization.set_sensitive(true);
			transport::read(
				self.agent
					.post(format!(
						"{}{}{name}",
						self.origin,
						format_args!("{}/", discovery.rpc.trim_end_matches('/'))
					))
					.header("Authorization", authorization)
					.send_json(args),
			)
		};
		let reply = send(&token)?;
		if reply.status == 401 && self.api_token.is_none() {
			let fresh = self.access(Some(&token))?;
			return send(&fresh)?.result(&[&token, &fresh]);
		}
		reply.result(&[&token])
	}
}

macro_rules! operations {
	($($method:ident : $module:ident :: $args:ident => $result:ty),* $(,)?) => {
		impl Client { $(
			#[doc = concat!("Calls `", stringify!($method), "` with the control plane's wire types.")]
			///
			/// # Errors
			/// Network, authentication, or a typed [`Rejected`] control-plane error.
			pub fn $method(&self, args: $module::$args) -> Result<$result> {
				self.call(stringify!($method), &args)
			}
		)* }
	};
}

operations! {
	whoami: account::Whoami => account::WhoAmI,
	organizations: account::Organizations => Vec<account::OrganizationSummary>,
	create_organization: account::CreateOrganization => account::OrganizationSummary,
	members: account::Members => Vec<account::Member>,
	invite_member: account::InviteMember => account::PendingInvitation,
	invitations: account::Invitations => Vec<account::PendingInvitation>,
	accept_invitation: account::AcceptInvitation => account::OrganizationSummary,
	revoke_invitation: account::RevokeInvitation => (),
	set_member_role: account::SetMemberRole => (),
	remove_member: account::RemoveMember => (),
	projects: projects::Projects => Vec<projects::ProjectSummary>,
	project: projects::Project => projects::ProjectDetail,
	create_project: projects::CreateProject => projects::CreatedProject,
	delete_project: projects::DeleteProject => (),
	environments: projects::Environments => Vec<projects::EnvironmentSummary>,
	create_environment: projects::CreateEnvironment => projects::EnvironmentSummary,
	update_environment: projects::UpdateEnvironment => projects::EnvironmentSummary,
	delete_environment: projects::DeleteEnvironment => (),
	tokens: tokens::Tokens => Vec<tokens::TokenSummary>,
	create_token: tokens::CreateToken => tokens::CreatedToken,
	revoke_token: tokens::RevokeToken => (),
}
