//! The operator's push configuration: which transports this service
//! speaks, from the environment `lk` passes through, and what each
//! provider needs to sign with.
//!
//! Every variable is operator-provided secret material, read once at
//! startup and held only in the transports' parsed keys. A provider with
//! some of its variables set and the rest missing refuses to start with
//! every missing or unusable piece named: half-configured is worse than
//! unconfigured, because it would look like push works until the first
//! send. A provider with nothing set is unconfigured, which is a normal
//! way to run — its platform is refused at enrollment and its inbox is
//! untouched.

/// APNs's two servers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApnsEnvironment {
	/// The development server, for builds run from a development topic.
	Sandbox,
	/// The production server.
	Production,
}

/// The APNs transport's configuration: the operator's `.p8` key, the ids
/// that name it, the app's topic, and which server to talk to.
#[derive(Clone)]
pub struct ApnsConfig {
	/// The `.p8` file's contents, PEM.
	pub key_p8: String,
	/// The key's id, from the same developer account page.
	pub key_id: String,
	/// The team that owns the key.
	pub team_id: String,
	/// The app's bundle id: the `apns-topic` every send carries.
	pub topic: String,
	/// Which server pushes go to.
	pub environment: ApnsEnvironment,
}

impl std::fmt::Debug for ApnsConfig {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ApnsConfig")
			.field("key_p8", &"<redacted>")
			.field("key_id", &self.key_id)
			.field("team_id", &self.team_id)
			.field("topic", &self.topic)
			.field("environment", &self.environment)
			.finish()
	}
}

/// The FCM transport's configuration: the operator's service-account
/// JSON file, whose project the sends go out under and whose key signs
/// the token exchange.
#[derive(Clone)]
pub struct FcmConfig {
	/// The service account JSON file's contents.
	pub service_account_json: String,
}

impl std::fmt::Debug for FcmConfig {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("FcmConfig")
			.field("service_account_json", &"<redacted>")
			.finish()
	}
}

/// The Web Push transport's configuration: the VAPID key pair the
/// ecosystem's own tools generate, the contact push services may write
/// to, and any extra push-service hosts the operator approved.
#[derive(Clone)]
pub struct WebPushConfig {
	/// The private key, 32 bytes, base64url.
	pub vapid_private_key: String,
	/// The public key, 65 bytes, base64url.
	pub vapid_public_key: String,
	/// A `mailto:` or `https:` contact.
	pub subject: String,
	/// Push-service hosts beyond the shipped ones, exact names, pushed
	/// to only over DNS-validated pinned connections.
	pub extra_hosts: Vec<String>,
}

impl std::fmt::Debug for WebPushConfig {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WebPushConfig")
			.field("vapid_private_key", &"<redacted>")
			.field("vapid_public_key", &self.vapid_public_key)
			.field("subject", &self.subject)
			.field("extra_hosts", &self.extra_hosts)
			.finish()
	}
}

/// The operator's push configuration, provider by provider. `None` is
/// unconfigured, which the enrollment and delivery paths answer clearly.
#[derive(Clone, Debug, Default)]
pub struct PushConfig {
	pub apns: Option<ApnsConfig>,
	pub fcm: Option<FcmConfig>,
	pub web: Option<WebPushConfig>,
}

impl PushConfig {
	/// Reads the operator's environment. Every variable's name is
	/// `LEPTOSKIT_NOTIFICATIONS_` plus its provider's part, exactly as the
	/// guide documents; nothing here resolves a secret, only reads what
	/// the process was given.
	///
	/// # Errors
	///
	/// Every missing or unusable piece of a partially-set provider,
	/// named, so an operator sees the whole fix at once.
	pub fn from_env() -> Result<Self, String> {
		Self::from_vars(std::env::vars())
	}

	/// Reads the operator's configuration from `vars` instead of the
	/// process's, as a test does.
	///
	/// # Errors
	///
	/// As [`from_env`](Self::from_env).
	pub fn from_vars<K: AsRef<str>, V: AsRef<str>>(
		vars: impl IntoIterator<Item = (K, V)>,
	) -> Result<Self, String> {
		let mut found: std::collections::HashMap<&'static str, String> =
			std::collections::HashMap::new();

		let names = [
			"APNS_KEY_P8",
			"APNS_KEY_ID",
			"APNS_TEAM_ID",
			"APNS_TOPIC",
			"APNS_ENVIRONMENT",
			"FCM_SERVICE_ACCOUNT",
			"WEB_PUSH_VAPID_PRIVATE_KEY",
			"WEB_PUSH_VAPID_PUBLIC_KEY",
			"WEB_PUSH_VAPID_SUBJECT",
			"WEB_PUSH_EXTRA_HOSTS",
		];

		for (key, value) in vars {
			let key = key.as_ref();
			let value = value.as_ref();

			for name in names {
				if key == format!("LEPTOSKIT_NOTIFICATIONS_{name}") {
					found.insert(name, value.to_owned());
				}
			}
		}

		let mut take =
			|name: &'static str| found.remove(name).filter(|value| !value.trim().is_empty());

		let apns = required(
			"apns",
			&[
				("APNS_KEY_P8", take("APNS_KEY_P8")),
				("APNS_KEY_ID", take("APNS_KEY_ID")),
				("APNS_TEAM_ID", take("APNS_TEAM_ID")),
				("APNS_TOPIC", take("APNS_TOPIC")),
				("APNS_ENVIRONMENT", take("APNS_ENVIRONMENT")),
			],
		)?
		.map(|values| {
			let environment = match values["APNS_ENVIRONMENT"].trim() {
				"sandbox" => ApnsEnvironment::Sandbox,
				"production" => ApnsEnvironment::Production,
				other => {
					return Err(format!(
						"LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT is {other:?}: use `sandbox` or \
						 `production`"
					));
				}
			};

			Ok::<ApnsConfig, String>(ApnsConfig {
				key_p8: values["APNS_KEY_P8"].clone(),
				key_id: values["APNS_KEY_ID"].clone(),
				team_id: values["APNS_TEAM_ID"].clone(),
				topic: values["APNS_TOPIC"].clone(),
				environment,
			})
		})
		.transpose()?;

		let fcm = required(
			"fcm",
			&[("FCM_SERVICE_ACCOUNT", take("FCM_SERVICE_ACCOUNT"))],
		)?
		.map(|values| {
			FcmConfig {
				service_account_json: values["FCM_SERVICE_ACCOUNT"].clone(),
			}
		});

		let web = required(
			"web push",
			&[
				(
					"WEB_PUSH_VAPID_PRIVATE_KEY",
					take("WEB_PUSH_VAPID_PRIVATE_KEY"),
				),
				(
					"WEB_PUSH_VAPID_PUBLIC_KEY",
					take("WEB_PUSH_VAPID_PUBLIC_KEY"),
				),
				("WEB_PUSH_VAPID_SUBJECT", take("WEB_PUSH_VAPID_SUBJECT")),
			],
		)?
		.map(|values| {
			let extra_hosts = take("WEB_PUSH_EXTRA_HOSTS")
				.unwrap_or_default()
				.split(',')
				.map(str::trim)
				.filter(|host| !host.is_empty())
				.map(str::to_owned)
				.collect();

			WebPushConfig {
				vapid_private_key: values["WEB_PUSH_VAPID_PRIVATE_KEY"].clone(),
				vapid_public_key: values["WEB_PUSH_VAPID_PUBLIC_KEY"].clone(),
				subject: values["WEB_PUSH_VAPID_SUBJECT"].clone(),
				extra_hosts,
			}
		});

		Ok(Self { apns, fcm, web })
	}
}

/// Whether a provider's variables are set whole: all of them, or none of
/// them. Some of them is an error naming every missing piece.
///
/// # Errors
///
/// The provider's missing variables, named with their full environment
/// names.
fn required(
	provider: &str,
	variables: &[(&'static str, Option<String>)],
) -> Result<Option<std::collections::HashMap<&'static str, String>>, String> {
	let set = variables
		.iter()
		.filter(|(_, value)| value.is_some())
		.count();

	if set == 0 {
		return Ok(None);
	}

	if set == variables.len() {
		let values = variables
			.iter()
			.filter_map(|(name, value)| value.as_ref().map(|value| (*name, value.to_owned())))
			.collect::<std::collections::HashMap<_, _>>();

		return Ok(Some(values));
	}

	let missing = variables
		.iter()
		.filter(|(_, value)| value.is_none())
		.map(|(name, _)| format!("LEPTOSKIT_NOTIFICATIONS_{name}"))
		.collect::<Vec<_>>()
		.join(", ");

	Err(format!(
		"the {provider} push transport is half configured: set all of its variables or none. \
		 Missing: {missing}"
	))
}
